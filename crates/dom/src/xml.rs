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

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use blitz_dom::{Attribute, LocalName, QualName, ns};
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
    /// The caller's stop check said so.
    Halted,
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
    /// Length of `Parser::declared` outside this element.
    scope: usize,
}

struct Parser<'d, 'a> {
    doc: &'d mut Document,
    src: &'a str,
    at: usize,
    /// Prefix to its namespaces, innermost last; `""` is the default. A hash
    /// lookup, since a tag can name thousands of prefixes in scope, and interned
    /// namespaces, so that a long one is held once however often it's used.
    scope: HashMap<&'a str, Vec<Namespace>>,
    /// The prefixes in `scope` in the order declared, for end tags to undo.
    declared: Vec<&'a str>,
    /// Text not placed yet: adjacent text and references make one node.
    text: String,
    /// Levels above the first level of content.
    base_depth: usize,
    max_depth: usize,
    /// The caller's stop check, and the work since it was asked.
    stop: &'a dyn Fn() -> bool,
    spent: Cell<u32>,
}

/// Units of work between asks of the stop check.
const STOP_EVERY: u32 = 256;
/// The bytes a scan of one long token goes through between looks at the stop.
const SCAN: usize = 64 * 1024;

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
    /// One step of `work` units: the time limit, if a call is running, and the
    /// caller's stop.
    fn tick(&self, work: usize) -> Parse<()> {
        self.doc.poll_by(work)?;
        let spent = self
            .spent
            .get()
            .saturating_add(u32::try_from(work).unwrap_or(u32::MAX))
            .saturating_add(1);
        if spent < STOP_EVERY {
            self.spent.set(spent);
            return Ok(());
        }
        self.spent.set(0);
        if (self.stop)() {
            return Err(Stop::Halted);
        }
        Ok(())
    }

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

    /// Skips white space, however much, looking at the stop as it goes.
    fn skip_space(&mut self) -> Parse<bool> {
        let start = self.at;
        loop {
            let bytes = &self.rest().as_bytes();
            let slice = &bytes[..bytes.len().min(SCAN)];
            let n = slice
                .iter()
                .take_while(|&&b| is_space(char::from(b)))
                .count();
            self.at += n;
            if n < SCAN {
                return Ok(self.at > start);
            }
            self.tick(n / 64)?;
        }
    }

    /// Where `s` first has a byte of `wanted`, in slices with the stop between them.
    fn scan_for(&self, s: &str, wanted: &[u8]) -> Parse<Option<usize>> {
        let mut base = 0;
        for slice in s.as_bytes().chunks(SCAN) {
            if let Some(i) = slice.iter().position(|b| wanted.contains(b)) {
                return Ok(Some(base + i));
            }
            base += slice.len();
            self.tick(slice.len() / 64)?;
        }
        Ok(None)
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
        // A long run pays for itself in the clock, and the stop is asked in slices.
        let mut rest = s;
        while !rest.is_empty() {
            let mut end = rest.len().min(SCAN);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let (piece, after) = rest.split_at(end);
            self.tick(piece.len() / 64)?;
            if let Some(c) = piece.chars().find(|&c| !is_xml_char(c)) {
                return bad(format!("U+{:04X} isn't allowed in XML", c as u32));
            }
            rest = after;
        }
        Ok(())
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

    fn namespace_of(&self, prefix: &str) -> Option<Namespace> {
        match self.scope.get(prefix).and_then(|l| l.last()) {
            Some(n) => Some(n.clone()),
            None if prefix == "xml" => Some(ns!(xml)),
            None => None,
        }
    }

    /// Ends the declarations made after `mark`.
    fn unscope(&mut self, mark: usize) {
        while self.declared.len() > mark {
            if let Some(prefix) = self.declared.pop()
                && let Some(list) = self.scope.get_mut(prefix)
            {
                list.pop();
                if list.is_empty() {
                    self.scope.remove(prefix);
                }
            }
        }
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
        let end = self.scan_for(rest, b"<")?.unwrap_or(rest.len());
        let run = &rest[..end];
        self.check_chars(run)?;
        if run.contains("]]>") {
            return bad("\"]]>\" in text");
        }
        self.at += end;
        let mut inner = run;
        while let Some(i) = self.scan_for(inner, b"&\r")? {
            self.tick(1)?;
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
        let spaced = self.skip_space()?;
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
            let spaced = self.skip_space()?;
            if self.eat("?>") {
                break;
            }
            if !spaced {
                return bad("malformed XML declaration");
            }
            let name = self.name()?;
            self.skip_space()?;
            self.need("=")?;
            self.skip_space()?;
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
        if !self.skip_space()? {
            return bad("malformed doctype");
        }
        let name = self.name()?;
        let spaced = self.skip_space()?;
        let (mut public_id, mut system_id) = ("", "");
        if spaced && self.eat("PUBLIC") {
            if !self.skip_space()? {
                return bad("malformed doctype");
            }
            public_id = self.quoted()?;
            if !self.skip_space()? {
                return bad("malformed doctype");
            }
            system_id = self.quoted()?;
        } else if spaced && self.eat("SYSTEM") {
            if !self.skip_space()? {
                return bad("malformed doctype");
            }
            system_id = self.quoted()?;
        }
        self.skip_space()?;
        if self.eat("[") {
            self.internal_subset()?;
            self.skip_space()?;
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
            self.tick(0)?;
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
        let scope_before = self.declared.len();
        let empty = loop {
            let spaced = self.skip_space()?;
            if self.eat("/>") {
                break true;
            }
            if self.eat(">") {
                break false;
            }
            if !spaced {
                return bad(format!("malformed tag at byte {}", self.at));
            }
            self.tick(0)?;
            let attr = self.name()?;
            self.skip_space()?;
            self.need("=")?;
            self.skip_space()?;
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
            Some(p) => match self.namespace_of(p) {
                Some(u) => u,
                None => return bad(format!("the prefix {p:?} isn't declared")),
            },
            None => self.namespace_of("").unwrap_or(ns!()),
        };
        let mut attrs = Vec::with_capacity(raw.len());
        let mut expanded: HashSet<(Namespace, &'a str)> = HashSet::new();
        for (attr, value) in raw {
            self.tick(0)?;
            let (p, l) =
                split_qname(attr).ok_or_else(|| Stop::Malformed(format!("bad name {attr:?}")))?;
            let uri = if attr == "xmlns" || p == Some("xmlns") {
                ns!(xmlns)
            } else if let Some(p) = p {
                match self.namespace_of(p) {
                    Some(u) => u,
                    None => return bad(format!("the prefix {p:?} isn't declared")),
                }
            } else {
                ns!()
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
        let qual = QualName::new(prefix.map(Prefix::from), namespace, LocalName::from(local));
        let id = self
            .doc
            .create_element_with(qual, attrs)
            .map_err(|e| self.doc.fail(e))?;
        self.doc.place_quietly(parent, &[id], None);
        if empty {
            self.unscope(scope_before);
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
        self.scope
            .entry(prefix)
            .or_default()
            .push(Namespace::from(uri));
        self.declared.push(prefix);
        Ok(())
    }

    /// One element and all it holds; `<` taken. Fragments go through
    /// [`content`](Self::content) with no open element.
    fn content(&mut self, top: usize, mode: Mode) -> Parse<()> {
        let mut open: Vec<Open<'a>> = Vec::new();
        let mut roots = 0usize;
        loop {
            self.tick(0)?;
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
                } else if !self.skip_space()? {
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
                self.skip_space()?;
                self.need(">")?;
                if name != o.name {
                    return bad(format!("</{name}> closes <{}>", o.name));
                }
                self.flush_text(o.id)?;
                self.unscope(o.scope);
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
                let scope = self.declared.len();
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
            self.tick(0)?;
            self.skip_space()?;
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
        self.skip_space()?;
        if self.rest().is_empty() {
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
/// made in, with `seed` as the namespaces in scope, one per prefix. `depth` is
/// how deep `top`'s children sit.
pub(crate) fn parse<'a>(
    doc: &mut Document,
    top: usize,
    src: &'a str,
    mode: Mode,
    seed: &'a HashMap<String, Namespace>,
    depth: usize,
    stop: &'a dyn Fn() -> bool,
) -> Parse<()> {
    let max_depth = doc.max_depth();
    let mut p = Parser {
        doc,
        src,
        at: 0,
        scope: seed
            .iter()
            .map(|(prefix, uri)| (prefix.as_str(), vec![uri.clone()]))
            .collect(),
        declared: Vec::new(),
        text: String::new(),
        base_depth: depth,
        max_depth,
        stop,
        spent: Cell::new(0),
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

/// The algorithm's "namespace prefix map", kept once for the whole walk: what
/// an element adds is undone at its end tag, so nothing is copied per element,
/// and a lookup is a hash, not a scan. Strings are borrowed from the document.
#[derive(Default)]
struct PrefixMap<'d> {
    /// Namespace to its prefixes, most recent last.
    by_namespace: HashMap<Option<&'d str>, Vec<Cow<'d, str>>>,
    /// How many times each pair is in `by_namespace`.
    pairs: HashMap<(Option<&'d str>, Cow<'d, str>), u32>,
    /// What was added, in order, for `undo`.
    added: Vec<(Option<&'d str>, Cow<'d, str>)>,
    /// How many namespaces each prefix stands for in scope.
    prefixes: HashMap<Cow<'d, str>, u32>,
}

impl<'d> PrefixMap<'d> {
    fn add(&mut self, ns: Option<&'d str>, prefix: Cow<'d, str>) {
        self.by_namespace
            .entry(ns)
            .or_default()
            .push(prefix.clone());
        *self.pairs.entry((ns, prefix.clone())).or_default() += 1;
        *self.prefixes.entry(prefix.clone()).or_default() += 1;
        self.added.push((ns, prefix));
    }

    /// Whether `prefix` stands for a namespace in scope.
    fn is_bound(&self, prefix: &str) -> bool {
        self.prefixes.contains_key(prefix)
    }

    fn has(&self, ns: Option<&'d str>, prefix: Cow<'d, str>) -> bool {
        self.pairs.contains_key(&(ns, prefix))
    }

    /// §3.2.1.1 "retrieve a preferred prefix string".
    fn preferred(&self, ns: Option<&'d str>, prefix: Option<&'d str>) -> Option<Cow<'d, str>> {
        if let Some(p) = prefix
            && self.has(ns, Cow::Borrowed(p))
        {
            return Some(Cow::Borrowed(p));
        }
        self.by_namespace.get(&ns).and_then(|l| l.last()).cloned()
    }

    /// Back to `mark`, a length of `added`.
    fn undo(&mut self, mark: usize) {
        while self.added.len() > mark {
            let Some((ns, prefix)) = self.added.pop() else {
                return;
            };
            if let Some(list) = self.by_namespace.get_mut(&ns) {
                list.pop();
                if list.is_empty() {
                    self.by_namespace.remove(&ns);
                }
            }
            if let Some(n) = self.prefixes.get_mut(&prefix) {
                *n -= 1;
                if *n == 0 {
                    self.prefixes.remove(&prefix);
                }
            }
            let key = (ns, prefix);
            if let Some(n) = self.pairs.get_mut(&key) {
                *n -= 1;
                if *n == 0 {
                    self.pairs.remove(&key);
                }
            }
        }
    }
}

/// Why a serialization stopped.
#[derive(Debug)]
pub(crate) enum Refused {
    /// The time limit.
    Time(BindingError),
    /// More than this many bytes: the output is outside the JS heap, and
    /// repeated namespace declarations can make it far larger than the tree.
    TooLong(usize),
}

impl From<BindingError> for Refused {
    fn from(e: BindingError) -> Self {
        Self::Time(e)
    }
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

enum Step<'d> {
    /// A node, and the namespace it inherits.
    Open(usize, Option<&'d str>),
    /// An end tag: the name, and where the map goes back to.
    Close(String, usize),
}

/// One serialization's state.
struct Serializer<'d> {
    doc: &'d Document,
    map: PrefixMap<'d>,
    index: u32,
    out: String,
    max_bytes: usize,
}

impl<'d> Serializer<'d> {
    fn check_size(&self) -> Result<(), Refused> {
        if self.out.len() > self.max_bytes {
            return Err(Refused::TooLong(self.max_bytes));
        }
        Ok(())
    }

    fn generate_prefix(&mut self) -> String {
        let g = format!("ns{}", self.index);
        self.index += 1;
        g
    }

    fn declaration(&mut self, prefix: Option<&str>, namespace: &str) {
        self.out.push_str(" xmlns");
        if let Some(p) = prefix {
            self.out.push(':');
            self.out.push_str(p);
        }
        self.out.push_str("=\"");
        escape_attribute(namespace, &mut self.out);
        self.out.push('"');
    }

    /// One element's start tag, per §3.2.1.1 steps 1 to 20. Returns the
    /// namespace its children inherit and the name of its end tag, if it has
    /// one. Attributes are counted against the clock and the size one by one.
    fn open_element(
        &mut self,
        id: usize,
        inherited: Option<&'d str>,
    ) -> Result<(Option<&'d str>, Option<String>), Refused> {
        let doc = self.doc;
        let Some(name) = doc.element_name(id) else {
            return Ok((inherited, None));
        };
        let attrs: &'d [Attribute] = doc.node(id).attrs().unwrap_or(&[]);
        let namespace = Some(name.ns.as_ref()).filter(|n| !n.is_empty());
        let local = name.local.as_ref();
        let element_prefix = name.prefix.as_deref();
        let mut local_prefixes: HashMap<&'d str, &'d str> = HashMap::new();
        // §3.2.1.1.1 "recording the namespace information".
        let mut local_default: Option<&'d str> = None;
        for a in attrs {
            doc.poll()?;
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
                    if !self.map.has(value, Cow::Borrowed(prefix)) {
                        self.map.add(value, Cow::Borrowed(prefix));
                    }
                    local_prefixes.insert(prefix, a.value.as_str());
                }
            }
        }
        let mut inherited_child = inherited;
        let mut ignore_default = false;
        let qualified;
        let mut declare: Option<(Option<Cow<'d, str>>, &'d str)> = None;
        if inherited == namespace {
            if local_default.is_some() {
                ignore_default = true;
            }
            qualified = if namespace == Some(XML) {
                format!("xml:{local}")
            } else {
                local.to_owned()
            };
        } else if let Some(c) = self.map.preferred(namespace, element_prefix) {
            qualified = format!("{c}:{local}");
            if let Some(d) = local_default.filter(|&d| d != XML) {
                inherited_child = Some(d).filter(|v| !v.is_empty());
            }
        } else if let Some(p) = element_prefix {
            let prefix = if local_prefixes.contains_key(p) {
                Cow::Owned(self.generate_prefix())
            } else {
                Cow::Borrowed(p)
            };
            self.map.add(namespace, prefix.clone());
            qualified = format!("{prefix}:{local}");
            declare = Some((Some(prefix), namespace.unwrap_or("")));
            if let Some(d) = local_default {
                inherited_child = Some(d).filter(|v| !v.is_empty());
            }
        } else if local_default != Some(namespace.unwrap_or("")) {
            ignore_default = true;
            qualified = local.to_owned();
            inherited_child = namespace;
            declare = Some((None, namespace.unwrap_or("")));
        } else {
            qualified = local.to_owned();
            inherited_child = namespace;
        }
        self.out.push('<');
        self.out.push_str(&qualified);
        if let Some((prefix, uri)) = declare {
            self.declaration(prefix.as_deref(), uri);
        }
        // §3.2.1.7 "serialization of the attributes".
        for a in attrs {
            doc.poll_by(1 + a.value.len() / 64)?;
            self.check_size()?;
            let attr_ns = Some(a.name.ns.as_ref()).filter(|n| !n.is_empty());
            let mut candidate: Option<Cow<'d, str>> = None;
            if let Some(ans) = attr_ns {
                candidate = self.map.preferred(attr_ns, a.name.prefix.as_deref());
                if ans == XMLNS {
                    let local_name = a.name.local.as_ref();
                    let known = local_prefixes.get(local_name) == Some(&a.value.as_str());
                    if a.value == XML
                        || (a.name.prefix.is_none() && ignore_default)
                        || (a.name.prefix.is_some()
                            && !known
                            && self.map.has(Some(&a.value), Cow::Borrowed(local_name)))
                    {
                        continue;
                    }
                    if a.name.prefix.as_deref() == Some("xmlns") {
                        candidate = Some(Cow::Borrowed("xmlns"));
                    }
                } else if candidate.is_none() {
                    // WPT XMLSerializer-serializeToString.html: not a prefix
                    // an ancestor binds to another namespace either.
                    let free = a
                        .name
                        .prefix
                        .as_deref()
                        .filter(|p| !local_prefixes.contains_key(p) && !self.map.is_bound(p));
                    let prefix = match free {
                        Some(p) => Cow::Borrowed(p),
                        None => Cow::Owned(self.generate_prefix()),
                    };
                    self.map.add(attr_ns, prefix.clone());
                    self.declaration(Some(&prefix), ans);
                    candidate = Some(prefix);
                }
            }
            self.out.push(' ');
            if let Some(c) = &candidate {
                self.out.push_str(c);
                self.out.push(':');
            }
            self.out.push_str(&a.name.local);
            self.out.push_str("=\"");
            escape_attribute(&a.value, &mut self.out);
            self.out.push('"');
        }
        self.check_size()?;
        let html = namespace == Some(HTML);
        let empty = doc.children(id).is_empty() && !(html && doc.is_html_template(id));
        if empty && html && VOID_HTML.contains(&local) {
            self.out.push_str(" />");
            return Ok((inherited_child, None));
        }
        if empty && !html {
            self.out.push_str("/>");
            return Ok((inherited_child, None));
        }
        self.out.push('>');
        Ok((inherited_child, Some(qualified)))
    }

    fn run(&mut self, top: usize) -> Result<(), Refused> {
        let doc = self.doc;
        let mut stack = vec![Step::Open(top, None)];
        while let Some(step) = stack.pop() {
            let (id, inherited) = match step {
                Step::Close(name, mark) => {
                    self.out.push_str("</");
                    self.out.push_str(&name);
                    self.out.push('>');
                    self.map.undo(mark);
                    continue;
                }
                Step::Open(id, inherited) => (id, inherited),
            };
            doc.poll()?;
            self.check_size()?;
            match doc.kind(id) {
                Some(Kind::Element) => {
                    let mark = self.map.added.len();
                    let (child_ns, close) = self.open_element(id, inherited)?;
                    match close {
                        Some(name) => {
                            stack.push(Step::Close(name, mark));
                            let kids: &[usize] = if doc.is_html_template(id) {
                                doc.template_of(id).map_or(&[], |c| doc.children(c))
                            } else {
                                doc.children(id)
                            };
                            stack.extend(kids.iter().rev().map(|&k| Step::Open(k, child_ns)));
                        }
                        None => self.map.undo(mark),
                    }
                }
                Some(Kind::Document | Kind::DocumentFragment) => {
                    let kids = doc.children(id);
                    stack.extend(kids.iter().rev().map(|&k| Step::Open(k, inherited)));
                }
                Some(Kind::Text) if doc.is_cdata(id) => {
                    self.out.push_str("<![CDATA[");
                    self.out.push_str(doc.char_data(id));
                    self.out.push_str("]]>");
                }
                Some(Kind::Text) => escape_text(doc.char_data(id), &mut self.out),
                Some(Kind::Comment) => {
                    self.out.push_str("<!--");
                    self.out.push_str(doc.char_data(id));
                    self.out.push_str("-->");
                }
                Some(Kind::ProcessingInstruction) => {
                    if let Some(Special::Instruction { target }) = doc.special(id) {
                        self.out.push_str("<?");
                        self.out.push_str(target);
                        self.out.push(' ');
                        self.out.push_str(doc.char_data(id));
                        self.out.push_str("?>");
                    }
                }
                Some(Kind::DocumentType) => {
                    if let Some(Special::Doctype {
                        name,
                        public_id,
                        system_id,
                    }) = doc.special(id)
                    {
                        let out = &mut self.out;
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
        self.check_size()
    }
}

/// §3.2 "XML serialization", with the require well-formed flag unset, as
/// `XMLSerializer` calls it. Iterative, with nothing copied per node: the
/// namespaces on its stack are borrowed from the document. The output stops at
/// [`DomLimits::max_text_bytes`](crate::DomLimits::max_text_bytes), since
/// repeated declarations can make it far larger than the tree.
pub(crate) fn serialize(doc: &Document, top: usize) -> Result<String, Refused> {
    let mut map = PrefixMap::default();
    map.add(Some(XML), Cow::Borrowed("xml"));
    let mut s = Serializer {
        doc,
        map,
        index: 1,
        out: String::new(),
        max_bytes: doc.max_text_bytes(),
    };
    s.run(top)?;
    Ok(s.out)
}
