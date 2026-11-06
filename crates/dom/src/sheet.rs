//! The little of CSSOM a page gets: `sheet`, `cssRules`, `insertRule`, `deleteRule`.
//!
//! CSS-in-JS writes rules with `insertRule` into a `<style>` whose text stays empty,
//! so nothing would reach the printer unless the output carries them. A sheet's
//! rules are read from the text with cssparser's tokens, the way the printer's
//! parser divides them, and stylo never sees them here: a stylo parse can't be
//! stopped. The script's view is in `glue_sheet.js`, in the heap. A sheet that has
//! been edited has its rules here as well, charged, and `to_html` writes those in
//! place of the element's text until its text next changes: a browser makes a new
//! sheet then, even for the same text, so what tells is a generation, not a compare.

use cssparser::{Parser, ParserInput, Token};
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};

use crate::document::{Document, Kind};

/// What one rule is charged beyond its text.
const RULE_COST: usize = 64;
/// CSSOM's `type` of the rules whose place in a sheet is fixed.
const KIND_IMPORT: u16 = 3;
const KIND_NAMESPACE: u16 = 10;

/// A style element's edited rules.
pub(crate) struct EditedSheet {
    /// The element's generation they came from; see [`Document::style_changed`].
    generation: u32,
    /// What the rules are charged.
    cost: usize,
    rules: Vec<String>,
}

/// One top-level rule.
#[derive(Debug, PartialEq, Eq)]
struct Rule {
    /// CSSOM's `type`: 1 for a style rule, 0 for what has none.
    kind: u16,
    /// A style rule's selector, an at-rule's name and prelude.
    prelude: String,
    /// What is in the block, with `None` for a statement such as `@import`.
    body: Option<String>,
}

/// CSS Syntax ends what is open at the end of its input without an error. Written out
/// before anything else, that would swallow what follows, so `text` comes back with
/// each open comment, string, `url(` and bracket closed, and without an escape that
/// has nothing to escape. Linear, with a stack as deep as `text` is long.
fn closed(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut open: Vec<u8> = Vec::new();
    let mut i = 0;
    let mut end = bytes.len();
    let mut tail = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if i + 1 == bytes.len() {
                    end = i;
                }
                i += 2;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                match text[i + 2..].find("*/") {
                    Some(at) => i += at + 4,
                    None => {
                        tail.push_str("*/");
                        break;
                    }
                }
                continue;
            }
            quote @ (b'"' | b'\'') => {
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => {
                            tail.push(char::from(quote));
                            break;
                        }
                        Some(b'\\') if i + 1 == bytes.len() => {
                            end = i;
                            i += 1;
                        }
                        Some(b'\\') => i += 2,
                        // A newline ends a bad string, which is closed.
                        Some(b'\n' | b'\r' | 0x0C) => break,
                        Some(&c) => {
                            i += 1;
                            if c == quote {
                                break;
                            }
                        }
                    }
                }
                continue;
            }
            b'(' => {
                let unquoted = i >= 3
                    && text
                        .get(i - 3..i)
                        .is_some_and(|n| n.eq_ignore_ascii_case("url"))
                    && !bytes[i + 1..]
                        .iter()
                        .find(|b| !b.is_ascii_whitespace())
                        .is_some_and(|b| matches!(b, b'"' | b'\''));
                if unquoted {
                    i += 1;
                    loop {
                        match bytes.get(i) {
                            None => {
                                tail.push(')');
                                break;
                            }
                            Some(b'\\') if i + 1 == bytes.len() => {
                                end = i;
                                i += 1;
                            }
                            Some(b'\\') => i += 2,
                            Some(b')') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                    continue;
                }
                open.push(b')');
            }
            b'[' => open.push(b']'),
            b'{' => open.push(b'}'),
            c @ (b')' | b']' | b'}') => {
                if open.last() == Some(&c) {
                    open.pop();
                }
            }
            _ => {}
        }
        i += 1;
    }
    let mut out = String::with_capacity(end + tail.len() + open.len());
    out.push_str(&text[..end]);
    out.push_str(&tail);
    out.extend(open.iter().rev().map(|&c| char::from(c)));
    out
}

impl Rule {
    /// CSSOM "serialize a CSS rule", as far as text allows: nothing inside a
    /// block is parsed, so it stays as written.
    fn text(&self) -> String {
        match self.body.as_deref().map(closed).as_deref() {
            None => format!("{};", closed(&self.prelude)),
            Some("") => format!("{} {{ }}", self.prelude),
            Some(b) if b.ends_with(';') || b.ends_with('}') || b.ends_with("*/") => {
                format!("{} {{ {b} }}", self.prelude)
            }
            Some(b) => format!("{} {{ {b}; }}", self.prelude),
        }
    }
}

/// CSSOM's `type` for an at-rule, or `None` for a name CSS doesn't have, which a
/// parser drops.
fn at_rule_kind(name: &str) -> Option<u16> {
    Some(match name.to_ascii_lowercase().as_str() {
        "import" => 3,
        "media" => 4,
        "font-face" => 5,
        "page" => 6,
        "keyframes" | "-webkit-keyframes" => 7,
        "namespace" => 10,
        "counter-style" => 11,
        "supports" => 12,
        "font-feature-values" => 14,
        "container"
        | "layer"
        | "property"
        | "scope"
        | "starting-style"
        | "font-palette-values"
        | "position-try"
        | "view-transition" => 0,
        _ => return None,
    })
}

/// CSS Cascade and Namespaces: `@import` is a statement that comes before all but
/// other imports and `@layer` statements, and `@namespace` a statement after imports
/// and before all else. What is out of place is dropped.
fn in_order(rules: Vec<Rule>) -> Vec<Rule> {
    let (mut imports_over, mut namespaces_over) = (false, false);
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let keep = match rule.kind {
            KIND_IMPORT => !imports_over,
            KIND_NAMESPACE => !namespaces_over,
            _ => true,
        };
        match rule.kind {
            KIND_IMPORT => {}
            KIND_NAMESPACE => imports_over = true,
            0 if rule.body.is_none() => {}
            _ => {
                imports_over = true;
                namespaces_over = true;
            }
        }
        if keep {
            out.push(rule);
        }
    }
    out
}

/// Whether an at-rule of `kind` may take the shape it has: a statement for `@import`
/// and `@namespace`, a block for the rest, and `@layer`, of type 0, either.
fn shape_fits(kind: u16, name_is_layer: bool, has_block: bool) -> bool {
    match kind {
        KIND_IMPORT | KIND_NAMESPACE => !has_block,
        _ => has_block || name_is_layer,
    }
}

/// The rules of `text`, in order. What isn't a rule is dropped, as a parser does.
fn parse_rules(text: &str) -> Vec<Rule> {
    in_order(top_level_rules(text))
}

fn top_level_rules(text: &str) -> Vec<Rule> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut out = Vec::new();
    loop {
        let start = parser.position();
        let mut kind = Some(1);
        let mut first = true;
        let mut prelude_end = None;
        let mut body = None;
        let mut layer = false;
        let mut semicolon = false;
        loop {
            let at = parser.position();
            let token = match parser.next_including_whitespace_and_comments() {
                Ok(t) => t.clone(),
                Err(_) => {
                    // The end of the input ends a statement.
                    if kind != Some(1) && !first {
                        prelude_end = Some(parser.position());
                    }
                    break;
                }
            };
            match token {
                Token::WhiteSpace(_) | Token::Comment(_) | Token::CDO | Token::CDC if first => {}
                Token::AtKeyword(name) if first => {
                    first = false;
                    layer = name.eq_ignore_ascii_case("layer");
                    kind = at_rule_kind(&name);
                }
                Token::Semicolon if kind != Some(1) => {
                    prelude_end = Some(at);
                    break;
                }
                Token::Semicolon => {
                    first = false;
                    semicolon = true;
                }
                Token::CurlyBracketBlock => {
                    prelude_end = Some(at);
                    let mut from = None;
                    let parsed: Result<(), cssparser::ParseError<'_, ()>> = parser
                        .parse_nested_block(|p| {
                            let begin = p.position();
                            while p.next_including_whitespace_and_comments().is_ok() {}
                            from = Some(p.slice_from(begin).trim().to_owned());
                            Ok(())
                        });
                    if parsed.is_ok() {
                        body = from;
                    }
                    break;
                }
                _ => first = false,
            }
        }
        let Some(end) = prelude_end else {
            return out;
        };
        let prelude = clean(parser.slice(start..end));
        let fits = |k: u16| {
            if k == 1 {
                body.is_some() && !semicolon
            } else {
                shape_fits(k, layer, body.is_some())
            }
        };
        if !prelude.is_empty()
            && let Some(kind) = kind.filter(|&k| fits(k))
        {
            out.push(Rule {
                kind,
                prelude,
                body,
            });
        }
    }
}

/// A prelude with no surrounding space or comments or `<!--`.
fn clean(text: &str) -> String {
    let mut s = text.trim();
    loop {
        if let Some(r) = s.strip_prefix("<!--").or_else(|| s.strip_prefix("-->")) {
            s = r.trim_start();
        } else if let Some(r) = s.strip_prefix("/*") {
            s = r.split_once("*/").map_or("", |(_, b)| b).trim_start();
        } else {
            return s.to_owned();
        }
    }
}

/// `cssRules(text)` for the glue: each rule's kind, prelude and text, each after a
/// NUL. CSS Syntax makes a NUL U+FFFD, so none is in a field.
pub(crate) fn rules_for_script(text: &str) -> String {
    let mut out = String::new();
    for rule in parse_rules(&text.replace('\0', "\u{fffd}")) {
        out.push_str(&format!(
            "\0{}\0{}\0{}",
            rule.kind,
            rule.prelude,
            rule.text()
        ));
    }
    out
}

/// CSSOM "parse a CSS rule": the same three fields, or `None` unless `text` is
/// exactly one rule.
pub(crate) fn one_rule(text: &str) -> Option<String> {
    let mut rules = parse_rules(&text.replace('\0', "\u{fffd}"));
    let rule = rules.pop().filter(|_| rules.is_empty())?;
    Some(format!(
        "\0{}\0{}\0{}",
        rule.kind,
        rule.prelude,
        rule.text()
    ))
}

fn no_element() -> BindingError {
    BindingError::TypeError("not a style element".to_owned())
}

impl Document {
    /// What a sheet is read from: the text of the element's own Text children, as
    /// HTML's "child text content" says.
    pub(crate) fn style_text(&self, id: usize) -> String {
        let mut out = String::new();
        for &c in self.children(id) {
            if self.kind(c) == Some(Kind::Text) {
                out.push_str(self.char_data(c));
            }
        }
        out
    }

    /// A style element's sheet generation, tracked from now on. Each change of its
    /// children or of their text makes it a new sheet, so a page with no sheet read
    /// pays one `is_empty` per mutation.
    pub(crate) fn style_generation(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = self.node_of(this)?;
        Ok(*self.style_generations.entry(id).or_insert(1))
    }

    /// The text under style element `parent`, if it is one a sheet was read from,
    /// has changed. `parent` is any node.
    pub(crate) fn style_changed(&mut self, parent: usize) {
        if !self.style_generations.is_empty()
            && let Some(g) = self.style_generations.get_mut(&parent)
        {
            *g = g.wrapping_add(1);
        }
    }

    /// Makes the rules of style element `id` editable here, from its text.
    fn edited(&mut self, id: usize) -> BindingResult<()> {
        let generation = self.style_generations.get(&id).copied().unwrap_or(0);
        if self
            .sheets
            .get(&id)
            .is_some_and(|s| s.generation == generation)
        {
            return Ok(());
        }
        self.sheets_forget(id);
        let rules: Vec<String> = parse_rules(&self.style_text(id))
            .iter()
            .map(Rule::text)
            .collect();
        let cost = rules.iter().map(|r| r.len() + RULE_COST).sum::<usize>();
        if self.charge_record(cost).is_none() {
            return Err(no_element());
        }
        self.sheets.insert(
            id,
            EditedSheet {
                generation,
                cost,
                rules,
            },
        );
        Ok(())
    }

    fn sheets_forget(&mut self, id: usize) {
        if let Some(old) = self.sheets.remove(&id) {
            self.credit_record(old.cost);
        }
    }

    /// `insertRule`, once the glue has checked it. `text` is a rule's own.
    pub(crate) fn sheet_insert(
        &mut self,
        this: ObjectId,
        index: u32,
        text: String,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.edited(id)?;
        let cost = text.len() + RULE_COST;
        if self.charge_record(cost).is_none() {
            return Err(no_element());
        }
        let sheet = self.sheets.get_mut(&id).ok_or_else(no_element)?;
        let at = (index as usize).min(sheet.rules.len());
        sheet.rules.insert(at, text);
        sheet.cost += cost;
        Ok(())
    }

    /// `deleteRule`.
    pub(crate) fn sheet_delete(&mut self, this: ObjectId, index: u32) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.edited(id)?;
        let sheet = self.sheets.get_mut(&id).ok_or_else(no_element)?;
        if (index as usize) < sheet.rules.len() {
            let gone = sheet.rules.remove(index as usize);
            let cost = gone.len() + RULE_COST;
            sheet.cost -= cost;
            self.credit_record(cost);
        }
        Ok(())
    }

    /// What `to_html` writes in a style element whose rules were edited, if nothing
    /// has changed its text since.
    pub(crate) fn edited_sheet_text(&self, id: usize) -> Option<String> {
        if self.sheets.is_empty() {
            return None;
        }
        let sheet = self.sheets.get(&id)?;
        let now = self.style_generations.get(&id).copied().unwrap_or(0);
        (sheet.generation == now).then(|| sheet.rules.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(u16, String)> {
        parse_rules(text)
            .into_iter()
            .map(|r| (r.kind, r.text()))
            .collect()
    }

    #[test]
    fn rules_divide_as_a_parser_divides_them() {
        assert_eq!(
            kinds(
                "@import 'u'; a{color:red}\n/* c */ b , i { top: 0; } @media print { x { y: z } }"
            ),
            vec![
                (3, "@import 'u';".to_owned()),
                (1, "a { color:red; }".to_owned()),
                (1, "b , i { top: 0; }".to_owned()),
                (4, "@media print { x { y: z } }".to_owned()),
            ]
        );
    }

    #[test]
    fn braces_in_strings_and_comments_dont_count() {
        assert_eq!(
            kinds("a::before { content: '}'; /* } */ top: 0 } b {}"),
            vec![
                (1, "a::before { content: '}'; /* } */ top: 0; }".to_owned()),
                (1, "b { }".to_owned()),
            ]
        );
    }

    #[test]
    fn what_isnt_a_rule_is_dropped() {
        assert!(kinds("").is_empty());
        assert!(kinds("nonsense").is_empty());
        assert!(kinds("{ a: b }").is_empty());
        assert!(kinds("@nonsense x { y }").is_empty());
        assert_eq!(kinds("a { b: c").len(), 1);
    }

    #[test]
    fn what_is_left_open_is_closed() {
        for (text, want) in [
            ("b{/*", "b { /**/ }"),
            ("b{top:0px/* c", "b { top:0px/* c*/ }"),
            ("b{content:\"x", "b { content:\"x\"; }"),
            ("b{content:'x\\", "b { content:'x'; }"),
            ("b{top:(1", "b { top:(1); }"),
            ("b{top:[(1", "b { top:[(1)]; }"),
            ("b{background:url(x", "b { background:url(x); }"),
            ("b{background:url( 'x", "b { background:url( 'x'); }"),
            (
                "@media print { a { top: 0px",
                "@media print { a { top: 0px} }",
            ),
            ("@import 'u", "@import 'u';"),
            ("b{top:0px\\", "b { top:0px; }"),
        ] {
            assert_eq!(parse_rules(text)[0].text(), want, "{text}");
        }
    }

    /// A rule that left something open is last in its own text, and with it
    /// closed nothing after it in a sheet is swallowed.
    #[test]
    fn nothing_after_a_closed_rule_is_swallowed() {
        for open in ["b{/*", "b{content:\"x", "b{top:(1", "b{top:url(x"] {
            let first = parse_rules(open)[0].text();
            let joined = format!("{first}\nc {{ top: 0px; }}");
            assert_eq!(kinds(&joined).len(), 2, "{open}: {joined}");
        }
    }

    #[test]
    fn closing_keeps_what_is_closed_already() {
        for text in [
            "a: b; /* c */ d: url(x) \"e\" (f) [g]",
            "a: '/*'; b: \"(\"",
            "url(/*)",
        ] {
            assert_eq!(closed(text), text);
        }
    }

    #[test]
    fn what_a_parser_drops_is_dropped() {
        let got = kinds("a;b{} c{} @media x; d{} @import 'u'; @namespace x 'y'; @import 'v' { }");
        let names: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(names, ["c { }", "d { }"]);
        let got = kinds("@layer a; @import 'u'; @namespace x 'y'; @import 'v'; b{}");
        let names: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            names,
            ["@layer a;", "@import 'u';", "@namespace x 'y';", "b { }"]
        );
    }

    #[test]
    fn deep_nesting_doesnt_recurse() {
        let deep = format!("a {{ b: {} }}", "(".repeat(1 << 20));
        assert_eq!(kinds(&deep).len(), 1);
        let curly = format!("a {}", "{".repeat(1 << 20));
        assert_eq!(kinds(&curly).len(), 1);
    }

    #[test]
    fn one_rule_means_one() {
        assert!(one_rule("a {}").is_some());
        assert!(one_rule("a {} b {}").is_none());
        assert!(one_rule("").is_none());
        assert!(one_rule("a").is_none());
    }
}
