//! A small XML parser: XML 1.0 with Namespaces, for `DOMParser`, XML documents'
//! `innerHTML` and pages served as XHTML.
//!
//! Hand-written so there is no new dependency, and small because nothing here
//! loads or expands anything. It is a non-validating processor that reads no
//! external entities (XML 1.0 §5.1 allows that): the internal DTD subset is
//! skipped, entities declared there aren't defined, and a reference to one is
//! a well-formedness error. That is a departure from browsers, in
//! `crates/dom/DESIGN.md`, and it is what makes an entity bomb impossible:
//! the output is at most as large as the input.
//!
//! Nothing recurses: the open elements are a vector. Nodes go through
//! [`Document`]'s constructors, so every limit applies as it does to a script's
//! nodes, and the clock is looked at per token.

use std::collections::HashSet;
use std::rc::Rc;

use blitz_dom::{Attribute, LocalName, QualName};
use html5ever::{Namespace, Prefix};
use surimono_webidl::runtime::BindingError;

use crate::document::{Kind, Special};
use crate::style_attr;
use crate::{Document, DomError};

const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
const HTML: &str = "http://www.w3.org/1999/xhtml";

/// Why a parse didn't finish.
#[derive(Debug)]
pub(crate) enum Stop {
    /// The input isn't well-formed: the message is for a `parsererror`
    /// document or a `SyntaxError`.
    Malformed(String),
    /// A limit, recorded with [`Document::fail`], or the time limit.
    Limit(BindingError),
}

impl From<BindingError> for Stop {
    fn from(e: BindingError) -> Self {
        Self::Limit(e)
    }
}

type Parse<T> = Result<T, Stop>;

fn bad<T>(what: impl Into<String>) -> Parse<T> {
    Err(Stop::Malformed(what.into()))
}

/// What is being parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A whole document, under its document node.
    Document,
    /// `element content` (XML 1.0 §3.1) under a fragment. The namespaces in
    /// scope come from the caller.
    Fragment,
}

/// One open element.
struct Open<'a> {
    id: usize,
    name: &'a str,
    /// Length of the namespace scope outside this element.
    scope: usize,
}

struct Parser<'d, 'a> {
    doc: &'d mut Document,
    src: &'a str,
    at: usize,
    /// (prefix, namespace) in scope, innermost last; `""` is the default.
    scope: Vec<(&'a str, String)>,
    /// Text not placed yet: adjacent text and references make one node.
    text: String,
    /// Levels above the first level of content.
    base_depth: usize,
    max_depth: usize,
}

fn is_name_start(c: char) -> bool {
    matches!(c, ':' | '_' | 'A'..='Z' | 'a'..='z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}'
            | '\u{203F}'..='\u{2040}')
}

/// XML 1.0 §2.2 `Char`.
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}'
        | '\u{10000}'..='\u{10FFFF}')
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Whether `name` matches the `Name` production.
pub(crate) fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_name_start) && chars.all(is_name_char)
}

/// Splits a `QName`. `None` if it isn't one (Namespaces §3).
fn split_qname(name: &str) -> Option<(Option<&str>, &str)> {
    match name.split_once(':') {
        None => Some((None, name)),
        Some((p, l)) if !p.is_empty() && !l.is_empty() && !l.contains(':') => Some((Some(p), l)),
        Some(_) => None,
    }
}

/// The entity or character reference with `body` between `&` and `;`
/// (XML 1.0 §4.1), pushed on `out`. Only the five predefined entities exist.
fn reference(body: &str, out: &mut String) -> Parse<()> {
    let c = match body {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let number = match (body.strip_prefix("#x"), body.strip_prefix('#')) {
                (Some(h), _) => u32::from_str_radix(h, 16),
                (None, Some(d)) => d.parse(),
                (None, None) => return bad(format!("undefined entity {body:?}")),
            };
            match number.ok().and_then(char::from_u32) {
                Some(c) if is_xml_char(c) && !body.contains('+') => c,
                _ => return bad(format!("bad character reference {body:?}")),
            }
        }
    };
    out.push(c);
    Ok(())
}

impl<'d, 'a> Parser<'d, 'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.at..]
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.at += s.len();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn skip_space(&mut self) -> bool {
        let rest = self.rest();
        let n = rest.len() - rest.trim_start_matches(is_space).len();
        self.at += n;
        n > 0
    }

    fn need(&mut self, s: &str) -> Parse<()> {
        if self.eat(s) {
            Ok(())
        } else {
            bad(format!("expected {s:?} at byte {}", self.at))
        }
    }

    fn name(&mut self) -> Parse<&'a str> {
        let rest = self.rest();
        let mut chars = rest.char_indices();
        match chars.next() {
            Some((_, c)) if is_name_start(c) => {}
            _ => return bad(format!("expected a name at byte {}", self.at)),
        }
        let end = chars
            .find(|&(_, c)| !is_name_char(c))
            .map_or(rest.len(), |(i, _)| i);
        self.at += end;
        Ok(&rest[..end])
    }

    /// The text up to `end`, which it consumes.
    fn until(&mut self, end: &str, what: &str) -> Parse<&'a str> {
        let rest = self.rest();
        let Some(i) = rest.find(end) else {
            return bad(format!("unterminated {what}"));
        };
        self.at += i + end.len();
        Ok(&rest[..i])
    }

    fn check_chars(&self, s: &str) -> Parse<()> {
        // A long run pays for itself in the clock.
        self.doc.poll_by(s.len() / 64)?;
        match s.chars().find(|&c| !is_xml_char(c)) {
            Some(c) => bad(format!("U+{:04X} isn't allowed in XML", c as u32)),
            None => Ok(()),
        }
    }

    fn quoted(&mut self) -> Parse<&'a str> {
        let quote = match self.peek() {
            Some(q @ ('"' | '\'')) => q,
            _ => return bad(format!("expected a quote at byte {}", self.at)),
        };
        self.at += 1;
        let q = if quote == '"' { "\"" } else { "'" };
        self.until(q, "literal")
    }

    /// An attribute value, normalized (XML 1.0 §3.3.3).
    fn attribute_value(&mut self) -> Parse<String> {
        let raw = self.quoted()?;
        self.check_chars(raw)?;
        if raw.contains('<') {
            return bad("\"<\" in an attribute value");
        }
        let mut out = String::with_capacity(raw.len());
        let mut inner = raw;
        while let Some(i) = inner.find(['&', '\t', '\n', '\r']) {
            out.push_str(&inner[..i]);
            let c = inner[i..].chars().next().unwrap_or(' ');
            inner = &inner[i + c.len_utf8()..];
            if c == '&' {
                let Some(end) = inner.find(';') else {
                    return bad("unterminated reference");
                };
                reference(&inner[..end], &mut out)?;
                inner = &inner[end + 1..];
            } else {
                // "\r\n" is one line end (§2.11), so one space.
                if c == '\r' && inner.starts_with('\n') {
                    inner = &inner[1..];
                }
                out.push(' ');
            }
        }
        out.push_str(inner);
        Ok(out)
    }

    fn namespace_of(&self, prefix: &str) -> Parse<Option<&str>> {
        self.doc.poll_by(self.scope.len() / 16)?;
        for (p, uri) in self.scope.iter().rev() {
            if *p == prefix {
                return Ok(Some(uri));
            }
        }
        Ok(match prefix {
            "xml" => Some(XML),
            _ => None,
        })
    }

    /// Text gathered so far becomes a node.
    fn flush_text(&mut self, parent: usize) -> Parse<()> {
        if self.text.is_empty() {
            return Ok(());
        }
        let text = std::mem::take(&mut self.text);
        let t = self.doc.create_text(&text).map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[t], None);
        Ok(())
    }

    /// Character data up to the next `<`, into `self.text`, line ends
    /// normalized (§2.11).
    fn char_data(&mut self) -> Parse<()> {
        let rest = self.rest();
        let end = rest.find('<').unwrap_or(rest.len());
        let run = &rest[..end];
        self.check_chars(run)?;
        if run.contains("]]>") {
            return bad("\"]]>\" in text");
        }
        self.at += end;
        let mut inner = run;
        while let Some(i) = inner.find(['&', '\r']) {
            self.text.push_str(&inner[..i]);
            let c = inner.as_bytes()[i];
            inner = &inner[i + 1..];
            if c == b'&' {
                let Some(stop) = inner.find(';') else {
                    return bad("unterminated reference");
                };
                reference(&inner[..stop], &mut self.text)?;
                inner = &inner[stop + 1..];
            } else {
                if inner.starts_with('\n') {
                    inner = &inner[1..];
                }
                self.text.push('\n');
            }
        }
        self.text.push_str(inner);
        Ok(())
    }

    fn comment(&mut self, parent: usize) -> Parse<()> {
        // `<!--` is taken.
        let body = self.until("--", "comment")?;
        self.check_chars(body)?;
        if !self.eat(">") {
            return bad("\"--\" in a comment");
        }
        let c = self
            .doc
            .create_comment(body)
            .map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[c], None);
        Ok(())
    }

    fn instruction(&mut self, parent: usize) -> Parse<()> {
        // `<?` is taken.
        let target = self.name()?;
        if target.eq_ignore_ascii_case("xml") || target.contains(':') {
            return bad(format!(
                "{target:?} can't be a processing instruction target"
            ));
        }
        let spaced = self.skip_space();
        let data = self.until("?>", "processing instruction")?;
        if !spaced && !data.is_empty() {
            return bad("a space must follow a processing instruction's target");
        }
        self.check_chars(data)?;
        let p = self
            .doc
            .create_instruction(target, data)
            .map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[p], None);
        Ok(())
    }

    fn cdata(&mut self, parent: usize) -> Parse<()> {
        // `<![CDATA[` is taken.
        let body = self.until("]]>", "CDATA section")?;
        self.check_chars(body)?;
        self.flush_text(parent)?;
        let text = body.replace("\r\n", "\n").replace('\r', "\n");
        let c = self.doc.create_cdata(&text).map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[c], None);
        Ok(())
    }

    /// `<?xml ...?>` (§2.8), checked for shape.
    fn declaration(&mut self) -> Parse<()> {
        if !self.rest().starts_with("<?xml")
            || !self.rest()[5..].chars().next().is_some_and(is_space)
        {
            return Ok(());
        }
        self.at += 5;
        let mut seen = 0;
        loop {
            let spaced = self.skip_space();
            if self.eat("?>") {
                break;
            }
            if !spaced {
                return bad("malformed XML declaration");
            }
            let name = self.name()?;
            self.skip_space();
            self.need("=")?;
            self.skip_space();
            let value = self.quoted()?;
            let (order, ok) = match name {
                "version" => (
                    0,
                    value
                        .strip_prefix("1.")
                        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
                ),
                "encoding" => (
                    1,
                    value.starts_with(|c: char| c.is_ascii_alphabetic())
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                ),
                "standalone" => (2, matches!(value, "yes" | "no")),
                _ => return bad(format!("{name:?} in the XML declaration")),
            };
            if !ok || order < seen || (seen == 0 && order != 0) {
                return bad("malformed XML declaration");
            }
            seen = order + 1;
        }
        if seen == 0 {
            return bad("the XML declaration has no version");
        }
        Ok(())
    }

    /// `<!DOCTYPE`, which is taken. The internal subset is skipped.
    fn doctype(&mut self, top: usize) -> Parse<()> {
        if !self.skip_space() {
            return bad("malformed doctype");
        }
        let name = self.name()?;
        let spaced = self.skip_space();
        let (mut public_id, mut system_id) = ("", "");
        if spaced && self.eat("PUBLIC") {
            if !self.skip_space() {
                return bad("malformed doctype");
            }
            public_id = self.quoted()?;
            if !self.skip_space() {
                return bad("malformed doctype");
            }
            system_id = self.quoted()?;
        } else if spaced && self.eat("SYSTEM") {
            if !self.skip_space() {
                return bad("malformed doctype");
            }
            system_id = self.quoted()?;
        }
        self.skip_space();
        if self.eat("[") {
            self.internal_subset()?;
            self.skip_space();
        }
        self.need(">")?;
        let t = self
            .doc
            .create_doctype(name, public_id, system_id)
            .map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(top, &[t], None);
        Ok(())
    }

    /// Up to the `]` that ends the internal subset, through quotes, comments
    /// and processing instructions.
    fn internal_subset(&mut self) -> Parse<()> {
        loop {
            self.doc.poll()?;
            if self.eat("<!--") {
                self.until("-->", "comment")?;
            } else if self.eat("<?") {
                self.until("?>", "processing instruction")?;
            } else {
                let rest = self.rest();
                let Some(i) = rest.find([']', '"', '\'', '<']) else {
                    return bad("unterminated doctype");
                };
                self.at += i;
                match rest.as_bytes()[i] {
                    b']' => {
                        self.at += 1;
                        return Ok(());
                    }
                    b'<' if self.rest().starts_with("<!--") || self.rest().starts_with("<?") => {}
                    b'<' => self.at += 1,
                    _ => {
                        self.quoted()?;
                    }
                }
            }
        }
    }

    /// A start tag, `<` taken. Returns the element and whether it is empty.
    fn start_tag(&mut self, parent: usize) -> Parse<(usize, &'a str, bool)> {
        let name = self.name()?;
        let mut raw: Vec<(&'a str, String)> = Vec::new();
        let mut seen: HashSet<&'a str> = HashSet::new();
        let scope_before = self.scope.len();
        let empty = loop {
            let spaced = self.skip_space();
            if self.eat("/>") {
                break true;
            }
            if self.eat(">") {
                break false;
            }
            if !spaced {
                return bad(format!("malformed tag at byte {}", self.at));
            }
            self.doc.poll()?;
            let attr = self.name()?;
            self.skip_space();
            self.need("=")?;
            self.skip_space();
            let value = self.attribute_value()?;
            if !seen.insert(attr) {
                return bad(format!("attribute {attr:?} appears twice"));
            }
            if raw.len() >= self.doc.max_attributes() {
                let limit = self.doc.max_attributes();
                let e = self.doc.fail(DomError::AttributeLimit { limit });
                return Err(Stop::Limit(e));
            }
            if attr == "xmlns" {
                self.declare("", &value)?;
            } else if let Some(prefix) = attr.strip_prefix("xmlns:") {
                self.declare(prefix, &value)?;
            }
            raw.push((attr, value));
        };
        let (prefix, local) =
            split_qname(name).ok_or_else(|| Stop::Malformed(format!("bad name {name:?}")))?;
        let namespace = match prefix {
            Some(p) => match self.namespace_of(p)? {
                Some(u) => u.to_owned(),
                None => return bad(format!("the prefix {p:?} isn't declared")),
            },
            None => self.namespace_of("")?.unwrap_or("").to_owned(),
        };
        let mut attrs = Vec::with_capacity(raw.len());
        let mut expanded: HashSet<(String, &'a str)> = HashSet::new();
        for (attr, value) in raw {
            self.doc.poll()?;
            let (p, l) =
                split_qname(attr).ok_or_else(|| Stop::Malformed(format!("bad name {attr:?}")))?;
            let uri = if attr == "xmlns" || p == Some("xmlns") {
                XMLNS.to_owned()
            } else if let Some(p) = p {
                match self.namespace_of(p)? {
                    Some(u) => u.to_owned(),
                    None => return bad(format!("the prefix {p:?} isn't declared")),
                }
            } else {
                String::new()
            };
            if !uri.is_empty() && !expanded.insert((uri.clone(), l)) {
                return bad(format!("attribute {attr:?} appears twice"));
            }
            if l == "style" && !style_attr::fits(&value) {
                let e = self.doc.fail(style_attr::limit_error());
                return Err(Stop::Limit(e));
            }
            let qual = QualName::new(
                p.map(Prefix::from),
                Namespace::from(uri),
                LocalName::from(l),
            );
            attrs.push(Attribute { name: qual, value });
        }
        let qual = QualName::new(
            prefix.map(Prefix::from),
            Namespace::from(namespace),
            LocalName::from(local),
        );
        let id = self
            .doc
            .create_element_with(qual, attrs)
            .map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[id], None);
        if empty {
            self.scope.truncate(scope_before);
        }
        Ok((id, name, empty))
    }

    fn declare(&mut self, prefix: &'a str, uri: &str) -> Parse<()> {
        let reserved = match prefix {
            "xmlns" => true,
            "xml" => uri != XML,
            _ => uri == XML || uri == XMLNS,
        };
        if reserved || (uri.is_empty() && !prefix.is_empty()) {
            return bad(format!("the prefix {prefix:?} can't be bound to {uri:?}"));
        }
        self.scope.push((prefix, uri.to_owned()));
        Ok(())
    }

    /// One element and all it holds; `<` taken. Fragments go through
    /// [`content`](Self::content) with no open element.
    fn content(&mut self, top: usize, mode: Mode) -> Parse<()> {
        let mut open: Vec<Open<'a>> = Vec::new();
        let mut roots = 0usize;
        loop {
            self.doc.poll()?;
            let parent = open.last().map_or(top, |o| o.id);
            let in_content = !open.is_empty() || mode == Mode::Fragment;
            if self.at >= self.src.len() {
                if !open.is_empty() {
                    return bad("unexpected end of input: an element is still open");
                }
                self.flush_text(parent)?;
                return Ok(());
            }
            if self.peek() != Some('<') {
                if in_content {
                    self.char_data()?;
                } else if !self.skip_space() {
                    return bad("text outside the document element");
                }
                continue;
            }
            self.at += 1;
            if self.eat("!--") {
                self.flush_text(parent)?;
                self.comment(parent)?;
            } else if self.eat("?") {
                self.flush_text(parent)?;
                self.instruction(parent)?;
            } else if self.eat("![CDATA[") {
                if !in_content {
                    return bad("a CDATA section outside the document element");
                }
                self.cdata(parent)?;
            } else if self.eat("/") {
                let Some(o) = open.pop() else {
                    return bad("an end tag with no start tag");
                };
                let name = self.name()?;
                self.skip_space();
                self.need(">")?;
                if name != o.name {
                    return bad(format!("</{name}> closes <{}>", o.name));
                }
                self.flush_text(o.id)?;
                self.scope.truncate(o.scope);
            } else {
                if !in_content && roots > 0 {
                    return bad("a second document element");
                }
                self.flush_text(parent)?;
                if self.base_depth + open.len() + 1 > self.max_depth {
                    let limit = self.max_depth;
                    let e = self.doc.fail(DomError::TreeTooDeep { limit });
                    return Err(Stop::Limit(e));
                }
                let scope = self.scope.len();
                let (id, name, empty) = self.start_tag(parent)?;
                if open.is_empty() {
                    roots += 1;
                }
                if !empty {
                    // The declarations stay until the end tag; `scope` is
                    // what they are cut back to.
                    open.push(Open { id, name, scope });
                }
            }
        }
    }

    fn document(&mut self, top: usize) -> Parse<()> {
        self.eat("\u{feff}");
        self.declaration()?;
        // The prolog's comments and instructions, up to a doctype or element.
        loop {
            self.doc.poll()?;
            self.skip_space();
            if self.eat("<!--") {
                self.comment(top)?;
            } else if self.rest().starts_with("<?") {
                self.at += 2;
                self.instruction(top)?;
            } else {
                break;
            }
        }
        if self.eat("<!DOCTYPE") {
            self.doctype(top)?;
        }
        if self.src[self.at..].trim_start_matches(is_space).is_empty() {
            return bad("no document element");
        }
        self.content(top, Mode::Document)?;
        if self.doc.document_element_of(top).is_none() {
            return bad("no document element");
        }
        Ok(())
    }
}

/// Parses `src` under `top`, a document or a fragment in the document being
/// made in, with `scope` as the namespaces in scope. `depth` is how deep
/// `top`'s children sit.
pub(crate) fn parse(
    doc: &mut Document,
    top: usize,
    src: &str,
    mode: Mode,
    scope: &[(String, String)],
    depth: usize,
) -> Parse<()> {
    let max_depth = doc.max_depth();
    let mut p = Parser {
        doc,
        src,
        at: 0,
        scope: scope
            .iter()
            .map(|(prefix, uri)| (prefix.as_str(), uri.clone()))
            .collect(),
        text: String::new(),
        base_depth: depth,
        max_depth,
    };
    match mode {
        Mode::Document => p.document(top),
        Mode::Fragment => p.content(top, mode),
    }
}

/// The document element of a document DOMParser couldn't parse (HTML §8.5.3).
pub(crate) fn error_name() -> QualName {
    QualName::new(
        None,
        Namespace::from("http://www.mozilla.org/newlayout/xml/parsererror.xml"),
        LocalName::from("parsererror"),
    )
}

// ------ XMLSerializer (DOM Parsing and Serialization §3.2) ------

/// Namespace to the prefixes bound to it, most recent last, as the algorithm's
/// "namespace prefix map". Cloned per element that declares something.
type PrefixMap = Vec<(Option<String>, Vec<String>)>;

fn map_add(map: &mut PrefixMap, ns: Option<&str>, prefix: &str) {
    match map.iter_mut().find(|(n, _)| n.as_deref() == ns) {
        Some((_, list)) => list.push(prefix.to_owned()),
        None => map.push((ns.map(str::to_owned), vec![prefix.to_owned()])),
    }
}

fn map_has(map: &PrefixMap, ns: Option<&str>, prefix: &str) -> bool {
    map.iter()
        .any(|(n, l)| n.as_deref() == ns && l.iter().any(|p| p == prefix))
}

/// §3.2.1.1 "retrieve a preferred prefix string".
fn preferred<'m>(map: &'m PrefixMap, ns: Option<&str>, prefix: Option<&str>) -> Option<&'m str> {
    let (_, list) = map.iter().find(|(n, _)| n.as_deref() == ns)?;
    let same = list.iter().find(|&x| Some(x.as_str()) == prefix);
    same.or(list.last()).map(String::as_str)
}

/// §3.2.1.5 "serializing an attribute value", which the current algorithm
/// also applies to tab and line ends so they survive a parse.
fn escape_attribute(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

fn escape_text(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

const VOID_HTML: [&str; 16] = [
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "link", "meta", "param", "source", "wbr",
];

enum Step {
    Open(usize, Option<String>, Rc<PrefixMap>),
    Close(String),
}

/// One element's start tag, per §3.2.1.1 steps 1 to 20. Returns the namespace
/// its children inherit, its map, and the name of its end tag, if it has one.
fn open_element(
    doc: &Document,
    id: usize,
    inherited: Option<String>,
    parent_map: &Rc<PrefixMap>,
    index: &mut u32,
    out: &mut String,
) -> (Option<String>, Rc<PrefixMap>, Option<String>) {
    let Some(name) = doc.element_name(id) else {
        return (inherited, parent_map.clone(), None);
    };
    let attrs = doc.node(id).attrs().unwrap_or(&[]);
    let namespace = Some(name.ns.as_ref()).filter(|n| !n.is_empty());
    let local = name.local.as_ref();
    let element_prefix = name.prefix.as_deref();
    let mut map = (**parent_map).clone();
    let mut local_prefixes: Vec<(&str, &str)> = Vec::new();
    // §3.2.1.1.1 "recording the namespace information".
    let mut local_default: Option<&str> = None;
    for a in attrs {
        if a.name.ns.as_ref() != XMLNS {
            continue;
        }
        match a.name.prefix.as_deref() {
            None => local_default = Some(a.value.as_str()),
            Some(_) => {
                let value = Some(a.value.as_str()).filter(|v| !v.is_empty());
                let prefix = a.name.local.as_ref();
                if value == Some(XML) || value.is_none() {
                    continue;
                }
                if !map_has(&map, value, prefix) {
                    map_add(&mut map, value, prefix);
                }
                local_prefixes.push((prefix, a.value.as_str()));
            }
        }
    }
    let mut inherited_child = inherited.clone();
    let mut ignore_default = false;
    let qualified;
    let mut declare = String::new();
    if inherited.as_deref() == namespace {
        if local_default.is_some() {
            ignore_default = true;
        }
        qualified = if namespace == Some(XML) {
            format!("xml:{local}")
        } else {
            local.to_owned()
        };
    } else {
        let candidate = preferred(&map, namespace, element_prefix).map(str::to_owned);
        if let Some(c) = candidate {
            qualified = format!("{c}:{local}");
            if let Some(d) = local_default.filter(|&d| d != XML) {
                inherited_child = Some(d.to_owned()).filter(|v| !v.is_empty());
            }
        } else if let Some(p) = element_prefix {
            let mut prefix = p.to_owned();
            if local_prefixes.iter().any(|(k, _)| *k == p) {
                prefix = format!("ns{index}");
                *index += 1;
            }
            map_add(&mut map, namespace, &prefix);
            qualified = format!("{prefix}:{local}");
            declare = format!(" xmlns:{prefix}=\"");
            escape_attribute(namespace.unwrap_or(""), &mut declare);
            declare.push('"');
            if let Some(d) = local_default {
                inherited_child = Some(d.to_owned()).filter(|v| !v.is_empty());
            }
        } else if local_default.is_none() || local_default != Some(namespace.unwrap_or("")) {
            ignore_default = true;
            qualified = local.to_owned();
            inherited_child = namespace.map(str::to_owned);
            declare = " xmlns=\"".to_owned();
            escape_attribute(namespace.unwrap_or(""), &mut declare);
            declare.push('"');
        } else {
            qualified = local.to_owned();
            inherited_child = namespace.map(str::to_owned);
        }
    }
    out.push('<');
    out.push_str(&qualified);
    out.push_str(&declare);
    // §3.2.1.7 "serialization of the attributes".
    for a in attrs {
        let attr_ns = Some(a.name.ns.as_ref()).filter(|n| !n.is_empty());
        let mut candidate: Option<String> = None;
        if let Some(ans) = attr_ns {
            candidate = preferred(&map, attr_ns, a.name.prefix.as_deref()).map(str::to_owned);
            if ans == XMLNS {
                let known = local_prefixes
                    .iter()
                    .any(|(k, v)| *k == a.name.local.as_ref() && *v == a.value);
                if a.value == XML
                    || (a.name.prefix.is_none() && ignore_default)
                    || (a.name.prefix.is_some()
                        && !known
                        && map_has(&map, Some(&a.value), a.name.local.as_ref()))
                {
                    continue;
                }
                if a.name.prefix.as_deref() == Some("xmlns") {
                    candidate = Some("xmlns".to_owned());
                }
            } else if candidate.is_none() {
                let mut p = a.name.prefix.as_deref().map(str::to_owned);
                let free = |p: &str| !local_prefixes.iter().any(|(k, _)| *k == p);
                if !p.as_deref().is_some_and(free) {
                    p = None;
                }
                let prefix = p.unwrap_or_else(|| {
                    let g = format!("ns{index}");
                    *index += 1;
                    g
                });
                map_add(&mut map, attr_ns, &prefix);
                out.push_str(" xmlns:");
                out.push_str(&prefix);
                out.push_str("=\"");
                escape_attribute(ans, out);
                out.push('"');
                candidate = Some(prefix);
            }
        }
        out.push(' ');
        if let Some(c) = &candidate {
            out.push_str(c);
            out.push(':');
        }
        out.push_str(&a.name.local);
        out.push_str("=\"");
        escape_attribute(&a.value, out);
        out.push('"');
    }
    let html = namespace == Some(HTML);
    let empty = doc.children(id).is_empty() && !(html && doc.is_html_template(id));
    if empty && html && VOID_HTML.contains(&local) {
        out.push_str(" />");
        return (inherited_child, Rc::new(map), None);
    }
    if empty && !html {
        out.push_str("/>");
        return (inherited_child, Rc::new(map), None);
    }
    out.push('>');
    (inherited_child, Rc::new(map), Some(qualified))
}

/// §3.2 "XML serialization", with the require well-formed flag unset, as
/// `XMLSerializer` calls it. Iterative. The polls are in the caller's hands
/// through `poll`, once per node.
pub(crate) fn serialize(doc: &Document, top: usize) -> Result<String, BindingError> {
    let xml_map: PrefixMap = vec![(Some(XML.to_owned()), vec!["xml".to_owned()])];
    let mut out = String::new();
    let mut index = 1u32;
    let mut stack = vec![Step::Open(top, None, Rc::new(xml_map))];
    while let Some(step) = stack.pop() {
        let (id, inherited, map) = match step {
            Step::Close(name) => {
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
                continue;
            }
            Step::Open(id, inherited, map) => (id, inherited, map),
        };
        doc.poll()?;
        match doc.kind(id) {
            Some(Kind::Element) => {
                let (child_ns, child_map, close) =
                    open_element(doc, id, inherited, &map, &mut index, &mut out);
                doc.poll_by(child_map.len())?;
                if let Some(name) = close {
                    stack.push(Step::Close(name));
                    let kids: &[usize] = if doc.is_html_template(id) {
                        doc.template_of(id).map_or(&[], |c| doc.children(c))
                    } else {
                        doc.children(id)
                    };
                    for &k in kids.iter().rev() {
                        stack.push(Step::Open(k, child_ns.clone(), child_map.clone()));
                    }
                }
            }
            Some(Kind::Document | Kind::DocumentFragment) => {
                for &k in doc.children(id).iter().rev() {
                    stack.push(Step::Open(k, inherited.clone(), map.clone()));
                }
            }
            Some(Kind::Text) if doc.is_cdata(id) => {
                out.push_str("<![CDATA[");
                out.push_str(doc.char_data(id));
                out.push_str("]]>");
            }
            Some(Kind::Text) => escape_text(doc.char_data(id), &mut out),
            Some(Kind::Comment) => {
                out.push_str("<!--");
                out.push_str(doc.char_data(id));
                out.push_str("-->");
            }
            Some(Kind::ProcessingInstruction) => {
                if let Some(Special::Instruction { target }) = doc.special(id) {
                    out.push_str("<?");
                    out.push_str(target);
                    out.push(' ');
                    out.push_str(doc.char_data(id));
                    out.push_str("?>");
                }
            }
            Some(Kind::DocumentType) => {
                if let Some(Special::Doctype {
                    name,
                    public_id,
                    system_id,
                }) = doc.special(id)
                {
                    out.push_str("<!DOCTYPE ");
                    out.push_str(name);
                    if !public_id.is_empty() {
                        out.push_str(" PUBLIC \"");
                        out.push_str(public_id);
                        out.push('"');
                    }
                    if !system_id.is_empty() {
                        if public_id.is_empty() {
                            out.push_str(" SYSTEM");
                        }
                        out.push_str(" \"");
                        out.push_str(system_id);
                        out.push('"');
                    }
                    out.push('>');
                }
            }
            None => {}
        }
    }
    Ok(out)
}
