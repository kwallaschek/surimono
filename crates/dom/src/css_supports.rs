//! `CSS.supports` (CSS Conditional 3 §8.1), for the `CSS` namespace in
//! `glue_window.js`; `CSS.escape` is all there, since it works on UTF-16.
//!
//! `supports` answers from stylo, the style engine the printer uses, so a page
//! that picks a fallback picks the one the printer can print. A property stylo
//! doesn't know (`fill`, `break-inside`, `backdrop-filter` in its Servo
//! configuration) takes only the CSS-wide keywords, though `element.style`
//! keeps any such declaration as text: a wrong `true` sends a page down a path
//! the printer can't follow, a wrong `false` only to its fallback.
//!
//! Stylo's answers hang on prefs blitz-dom sets when it makes a document
//! (grid, columns), so they are what the printer's stylo answers too.
//!
//! Bounded: stylo's parse can't be interrupted and recurses into blocks, so
//! the input is at most [`MAX_CHECKED_VALUE`] bytes and [`MAX_NESTING`]
//! brackets deep, checked here first in one bounded pass; past either the
//! answer is `false`.

use std::borrow::Cow;

use cssparser::{ParseError, Parser, ParserInput, Token};
use style::context::QuirksMode;
use style::parser::ParserContext;
use style::stylesheets::supports_rule::SupportsCondition;
use style::stylesheets::{CssRuleType, Origin};
use style_traits::ParsingMode;

use crate::sheet_screen::MAX_NESTING;
use crate::style_attr::{self, MAX_CHECKED_VALUE};

/// `CSS.supports(property, value)`.
pub(crate) fn supports_declaration(property: &str, value: &str) -> bool {
    if property.len() > MAX_CHECKED_VALUE || value.len() > MAX_CHECKED_VALUE || !shallow(value) {
        return false;
    }
    if let Some(name) = property.strip_prefix("--") {
        // css-variables-2: any `<declaration-value>`, which is at least one
        // token; `!important` isn't part of one.
        if name.is_empty() {
            return false;
        }
        return matches!(
            style_attr::parse_value(property, value, QuirksMode::NoQuirks),
            Ok(Some(text)) if !text.is_empty()
        );
    }
    // A legacy alias (`-webkit-transform`) is its property, as for
    // `element.style`.
    let property = style_attr::normalized(property);
    if !style_attr::known_to_stylo(&property) {
        // A property `element.style` has but stylo doesn't (`fill`) takes the
        // CSS-wide keywords, as every property does, and nothing stylo can't
        // check.
        return style_attr::supported(&property) && wide_keyword(value);
    }
    matches!(
        style_attr::parse_value(&property, value, QuirksMode::NoQuirks),
        Ok(Some(_))
    )
}

/// css-values-4 §7.3: a value every property takes, with whitespace around.
fn wide_keyword(value: &str) -> bool {
    let v = value.trim_matches(|c: char| c.is_ascii_whitespace());
    ["initial", "inherit", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
}

/// `CSS.supports(conditionText)`: a `<supports-condition>`, or failing that the
/// text in parentheses, as a declaration.
pub(crate) fn supports_condition(text: &str) -> bool {
    if text.len() > MAX_CHECKED_VALUE || !shallow(text) {
        return false;
    }
    let url = style_attr::base();
    let context = ParserContext::new(
        Origin::Author,
        url,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Cow::default(),
        None,
        None,
    );
    let eval = |text: &str| {
        let mut input = ParserInput::new(text);
        let mut p = Parser::new(&mut input);
        p.parse_entirely(SupportsCondition::parse)
            .is_ok_and(|c| c.eval(&context))
    };
    eval(text) || eval(&format!("({text})"))
}

/// Whether `text` nests at most [`MAX_NESTING`] blocks deep and tokenizes
/// with no error token. Recursion here stops at the cap.
fn shallow(text: &str) -> bool {
    fn walk<'i>(p: &mut Parser<'i, '_>, depth: usize) -> Result<(), ParseError<'i, ()>> {
        loop {
            let Ok(t) = p.next_including_whitespace_and_comments() else {
                return Ok(());
            };
            match t {
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    if depth + 1 >= MAX_NESTING {
                        return Err(p.new_custom_error(()));
                    }
                    p.parse_nested_block(|p| walk(p, depth + 1))?;
                }
                _ => {}
            }
        }
    }
    let mut input = ParserInput::new(text);
    walk(&mut Parser::new(&mut input), 0).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stylo's answers hang on prefs blitz-dom sets as it makes a document
    /// (`layout.grid.enabled` and the like), as it has by the time any
    /// script runs.
    fn with_a_document() {
        let _ = crate::Document::parse("");
    }

    #[test]
    fn declarations_as_stylo_takes_them() {
        with_a_document();
        // Chromium agrees on each of these.
        for (p, v, want) in [
            ("color", "red", true),
            ("COLOR", "red", true),
            ("color", "RED", true),
            ("color", "red;", false),
            ("color", "red; background: blue", false),
            ("color", "red !important", false),
            ("color", "  red  ", true),
            ("color", "", false),
            ("color", "rgb(1,2,3)", true),
            ("display", "grid", true),
            ("display", "flex", true),
            ("display", "foo", false),
            ("width", "calc(1px + 2px)", true),
            ("width", "1px)", false),
            ("foo", "bar", false),
            ("", "red", false),
            ("float", "left", true),
            ("float", "footnote", false),
            ("-webkit-transform", "none", true),
            ("-WEBKIT-TRANSFORM", "none", true),
            ("fill", "red", false),
            ("fill", "initial", true),
            ("fill", " INHERIT ", true),
            ("foo", "initial", false),
            ("--foo", "red", true),
            ("--foo", "", false),
            ("--foo", "!important", false),
            ("--foo", "red !important", false),
            ("--foo", "}", false),
            ("--foo", ")", false),
            ("--", "x", false),
        ] {
            assert_eq!(supports_declaration(p, v), want, "{p:?} {v:?}");
        }
    }

    #[test]
    fn conditions_as_stylo_takes_them() {
        with_a_document();
        for (c, want) in [
            ("(color: red)", true),
            ("color: red", true),
            ("color:red", true),
            ("(color: red) and (display: grid)", true),
            ("(color: red) or (foo: bar)", true),
            ("not (foo: bar)", true),
            ("not (color: red)", false),
            ("(color: red) and (foo: bar)", false),
            ("(foo)", false),
            ("foo(bar)", false),
            ("(color: red) (display: grid)", false),
            ("((color: red))", true),
            ("", false),
            (" ", false),
            ("(color: red", true),
            ("color: red;", false),
            ("--x: 1", true),
            ("(--x: 1)", true),
            ("(color: red) and not (foo: bar)", false),
            ("not not (color: red)", false),
            ("(color: red) AND (display: grid)", true),
            ("(COLOR: RED)", true),
            ("display: grid; color: red", false),
            ("(display: grid) and", false),
            ("and (display: grid)", false),
            ("selector(a > b)", true),
            ("selector(a,b)", false),
        ] {
            assert_eq!(supports_condition(c), want, "{c:?}");
        }
    }

    #[test]
    fn hostile_input_is_bounded() {
        for depth in [MAX_NESTING - 2, MAX_NESTING, 100_000] {
            let deep = format!("{}color: red{}", "(".repeat(depth), ")".repeat(depth));
            assert_eq!(supports_condition(&deep), depth < MAX_NESTING, "{depth}");
            let calc = format!("{}1px{}", "calc(".repeat(depth), ")".repeat(depth));
            let _ = supports_declaration("width", &calc);
            let sel = format!("selector({}a{})", ":is(".repeat(depth), ")".repeat(depth));
            let _ = supports_condition(&sel);
        }
        let big = "a".repeat(MAX_CHECKED_VALUE + 1);
        assert!(!supports_condition(&big));
        assert!(!supports_declaration("--x", &big));
        assert!(!supports_declaration(&big, "red"));
        for s in [
            "(", ")", "\"", "url(", "\\", "\0", "!", "{", "}", "@", "--", "(--:",
        ] {
            let _ = supports_condition(s);
            let _ = supports_declaration("color", s);
            let _ = supports_declaration(s, s);
        }
    }
}
