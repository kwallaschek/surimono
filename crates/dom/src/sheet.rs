//! The little of CSSOM a page gets: `sheet`, `cssRules`, `insertRule`, `deleteRule`.
//!
//! CSS-in-JS writes rules with `insertRule` into a `<style>` whose text stays empty,
//! so nothing would reach the printer unless the output carries them. A sheet's
//! rules are read from the text with cssparser's tokens, the way the printer's
//! parser divides them, and stylo never sees them here: a stylo parse can't be
//! stopped. The script's view is in `glue_sheet.js`, in the heap. A sheet that has
//! been edited has its rules here as well, charged, and `to_html` writes those in
//! place of the element's text for as long as the text is the one they came from
//! (a browser makes a new sheet when the text changes).

use cssparser::{Parser, ParserInput, Token};
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};

use crate::document::Document;

/// What one rule is charged beyond its text.
const RULE_COST: usize = 64;

/// A style element's edited rules.
pub(crate) struct EditedSheet {
    /// The text they came from.
    source: String,
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

impl Rule {
    /// CSSOM "serialize a CSS rule", as far as text allows: nothing inside a
    /// block is parsed, so it stays as written.
    fn text(&self) -> String {
        match &self.body {
            None => format!("{};", self.prelude),
            Some(b) if b.is_empty() => format!("{} {{ }}", self.prelude),
            Some(b) if b.ends_with(';') || b.ends_with('}') => {
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

/// The rules of `text`, in order. What isn't a rule is dropped, as a parser does.
fn parse_rules(text: &str) -> Vec<Rule> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut out = Vec::new();
    loop {
        let start = parser.position();
        let mut kind = Some(1);
        let mut first = true;
        let mut prelude_end = None;
        let mut body = None;
        loop {
            let at = parser.position();
            let token = match parser.next_including_whitespace_and_comments() {
                Ok(t) => t.clone(),
                Err(_) => break,
            };
            match token {
                Token::WhiteSpace(_) | Token::Comment(_) | Token::CDO | Token::CDC if first => {}
                Token::AtKeyword(name) if first => {
                    first = false;
                    kind = at_rule_kind(&name);
                }
                Token::Semicolon if kind != Some(1) => {
                    prelude_end = Some(at);
                    break;
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
        if !prelude.is_empty()
            && let Some(kind) = kind
            && (kind != 1 || body.is_some())
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
    /// The element's text, as a sheet comes from it.
    fn style_text(&self, id: usize) -> String {
        self.text_content(id).unwrap_or_default()
    }

    /// Makes the rules of style element `this` editable here, from its text.
    fn edited(&mut self, id: usize) -> BindingResult<()> {
        let source = self.style_text(id);
        if self.sheets.get(&id).is_some_and(|s| s.source == source) {
            return Ok(());
        }
        self.sheets_forget(id);
        let rules: Vec<String> = parse_rules(&source).iter().map(Rule::text).collect();
        let cost = source.len() + rules.iter().map(|r| r.len() + RULE_COST).sum::<usize>();
        if self.charge_record(cost).is_none() {
            return Err(no_element());
        }
        self.sheets.insert(id, EditedSheet { source, rules });
        Ok(())
    }

    fn sheets_forget(&mut self, id: usize) {
        if let Some(old) = self.sheets.remove(&id) {
            let cost =
                old.source.len() + old.rules.iter().map(|r| r.len() + RULE_COST).sum::<usize>();
            self.credit_record(cost);
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
        if self.charge_record(text.len() + RULE_COST).is_none() {
            return Err(no_element());
        }
        let sheet = self.sheets.get_mut(&id).ok_or_else(no_element)?;
        let at = (index as usize).min(sheet.rules.len());
        sheet.rules.insert(at, text);
        Ok(())
    }

    /// `deleteRule`.
    pub(crate) fn sheet_delete(&mut self, this: ObjectId, index: u32) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.edited(id)?;
        let sheet = self.sheets.get_mut(&id).ok_or_else(no_element)?;
        if (index as usize) < sheet.rules.len() {
            let gone = sheet.rules.remove(index as usize);
            self.credit_record(gone.len() + RULE_COST);
        }
        Ok(())
    }

    /// What `to_html` writes in a style element whose rules were edited, if its
    /// text is still the one they came from.
    pub(crate) fn edited_sheet_text(&self, id: usize) -> Option<String> {
        if self.sheets.is_empty() {
            return None;
        }
        let sheet = self.sheets.get(&id)?;
        (sheet.source == self.style_text(id)).then(|| sheet.rules.join("\n"))
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
                "a{color:red}\n/* c */ b , i { top: 0; } @media print { x { y: z } } @import 'u';"
            ),
            vec![
                (1, "a { color:red; }".to_owned()),
                (1, "b , i { top: 0; }".to_owned()),
                (4, "@media print { x { y: z } }".to_owned()),
                (3, "@import 'u';".to_owned()),
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
