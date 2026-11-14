//! Style text without the declarations only a paged medium has, for the
//! Raikiri engine's measuring copy (`layout_raikiri.rs`).
//!
//! Raikiri is a printer's layout: it knows CSS Generated Content for Paged
//! Media, which no browser does. A browser drops such a declaration as
//! unknown, so whatever an earlier declaration said still holds, and a script
//! measures the page laid out that way. The measuring copy gets the same
//! text a browser would keep:
//!
//! - `float: footnote` (the element stays in its line);
//! - `position: running(<name>)` (the element stays in the flow);
//! - a `content` with `string()`, `element()`, `leader()`,
//!   `target-counter()`, `target-counters()` or `target-text()` in it.
//!
//! Chromium 153 drops each of these: tests/data/chrome-layout/paged-only.
//! The text is read as tokens, never as words, so a string or a comment that
//! spells one of them is left alone. It is read after the sheet screen
//! (`sheet_screen.rs`) or `style_attr::parse` has bounded its nesting.

use std::borrow::Cow;
use std::ops::Range;

use cssparser::{ParseError, Parser, ParserInput, Token};

/// What the text has to spell, in any case, for a declaration to be taken
/// out of it: looked for first, so that almost every text is not read twice.
const NEEDLES: [&[u8]; 6] = [
    b"footnote",
    b"running(",
    b"string(",
    b"element(",
    b"leader(",
    b"target-",
];

/// The functions a browser doesn't know in `content`.
const CONTENT_FUNCTIONS: [&str; 6] = [
    "string",
    "element",
    "leader",
    "target-counter",
    "target-counters",
    "target-text",
];

/// `css` (a sheet, or the declarations of a style attribute) with the
/// declarations only a paged medium has taken out. Borrowed when there are
/// none, which is almost always.
pub(crate) fn unpaged(css: &str) -> Cow<'_, str> {
    let bytes = css.as_bytes();
    let spelled = |needle: &[u8]| {
        bytes
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    };
    if !NEEDLES.iter().any(|needle| spelled(needle)) {
        return Cow::Borrowed(css);
    }
    let mut cuts = Vec::new();
    let mut input = ParserInput::new(css);
    walk(&mut Parser::new(&mut input), &mut cuts);
    if cuts.is_empty() {
        return Cow::Borrowed(css);
    }
    let mut out = String::with_capacity(css.len());
    let mut from = 0;
    for cut in cuts {
        // Cuts come in source order and never overlap; a boundary that isn't
        // a character's is left alone (the tokenizer gives none).
        if cut.start < from || css.get(from..cut.start).is_none() || css.get(cut.end..).is_none() {
            continue;
        }
        out.push_str(&css[from..cut.start]);
        from = cut.end;
    }
    out.push_str(&css[from..]);
    Cow::Owned(out)
}

/// What has been read of one statement: a declaration, until a `{}` shows it
/// was a rule.
#[derive(Default)]
struct Statement {
    /// Tokens read, without white space and comments.
    tokens: usize,
    property: Option<Property>,
    colon: bool,
    /// Value tokens read, a function or a bracket with all it holds as one,
    /// `!important` not counted.
    values: usize,
    /// The value's first token is the one the property is dropped for.
    first_is_paged: bool,
    /// A function a browser doesn't know in `content` is in the value.
    paged_function: bool,
    /// The last value token was `!`.
    bang: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Property {
    Float,
    Position,
    Content,
}

impl Statement {
    fn read(&mut self, token: &Token<'_>) {
        self.tokens += 1;
        match (self.tokens, token) {
            (1, Token::Ident(name)) => {
                self.property = [
                    ("float", Property::Float),
                    ("position", Property::Position),
                    ("content", Property::Content),
                ]
                .into_iter()
                .find_map(|(n, p)| name.eq_ignore_ascii_case(n).then_some(p));
            }
            (2, Token::Colon) => self.colon = true,
            _ if self.colon => self.value(token),
            _ => {}
        }
    }

    fn value(&mut self, token: &Token<'_>) {
        if self.bang {
            self.bang = false;
            if matches!(token, Token::Ident(word) if word.eq_ignore_ascii_case("important")) {
                return;
            }
            // `!` and something else: a value of two more tokens.
            self.values += 1;
        }
        if matches!(token, Token::Delim('!')) {
            self.bang = true;
            return;
        }
        self.values += 1;
        match (self.property, token) {
            (Some(Property::Float), Token::Ident(word)) if self.values == 1 => {
                self.first_is_paged = word.eq_ignore_ascii_case("footnote");
            }
            (Some(Property::Position), Token::Function(name)) if self.values == 1 => {
                self.first_is_paged = name.eq_ignore_ascii_case("running");
            }
            (Some(Property::Content), Token::Function(name)) => {
                self.paged_function |= CONTENT_FUNCTIONS
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f));
            }
            _ => {}
        }
    }

    /// Whether this is a declaration a browser drops.
    fn is_paged(&self) -> bool {
        self.colon
            && match self.property {
                Some(Property::Float | Property::Position) => {
                    self.first_is_paged && self.values == 1
                }
                Some(Property::Content) => self.paged_function,
                None => false,
            }
    }
}

/// Reads the statements of one block (or of the whole text) and notes the
/// byte ranges of the declarations to take out, each without its `;`.
fn walk(p: &mut Parser<'_, '_>, cuts: &mut Vec<Range<usize>>) {
    let mut start = p.position().byte_index();
    let mut statement = Statement::default();
    loop {
        let before = p.position().byte_index();
        let Ok(token) = p.next_including_whitespace_and_comments() else {
            break;
        };
        let token = token.clone();
        match &token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Semicolon => {
                if statement.is_paged() {
                    cuts.push(start..before);
                }
                start = p.position().byte_index();
                statement = Statement::default();
            }
            Token::CurlyBracketBlock => {
                // A rule: its block holds statements of its own.
                nested(p, |p| walk(p, cuts));
                start = p.position().byte_index();
                statement = Statement::default();
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                statement.read(&token);
                let mut inner = false;
                nested(p, |p| inner = holds_content_function(p));
                statement.paged_function |=
                    inner && statement.property == Some(Property::Content) && statement.colon;
            }
            _ => statement.read(&token),
        }
    }
    if statement.is_paged() {
        cuts.push(start..p.position().byte_index());
    }
}

fn nested(p: &mut Parser<'_, '_>, read: impl FnOnce(&mut Parser<'_, '_>)) {
    let result: Result<(), ParseError<'_, ()>> = p.parse_nested_block(|p| {
        read(p);
        Ok(())
    });
    drop(result);
}

/// Reads to the end of a function's or a bracket's contents and says whether
/// one of [`CONTENT_FUNCTIONS`] is in them.
fn holds_content_function(p: &mut Parser<'_, '_>) -> bool {
    let mut found = false;
    while let Ok(token) = p.next() {
        let token = token.clone();
        match &token {
            Token::Function(name) => {
                found |= CONTENT_FUNCTIONS
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f));
                let mut inner = false;
                nested(p, |p| inner = holds_content_function(p));
                found |= inner;
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let mut inner = false;
                nested(p, |p| inner = holds_content_function(p));
                found |= inner;
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_paged_declarations_is_borrowed() {
        for css in [
            "",
            "a { float: left; position: absolute; content: 'x' }",
            // Spelled, but not declared.
            "a { content: 'float: footnote' } /* position: running(x) */",
            ".footnote { color: red } #running(x) { }",
            "a { --note: footnote; float: var(--note) }",
            "a { float: footnote left; position: running(x) y }",
            "a { background: element(#x) }",
        ] {
            assert!(matches!(unpaged(css), Cow::Borrowed(_)), "{css}");
        }
    }

    #[test]
    fn paged_declarations_are_taken_out_and_the_rest_is_kept() {
        for (css, want) in [
            ("float: footnote", ""),
            (
                "float:left;FLOAT : FootNote !important;color:red",
                "float:left;;color:red",
            ),
            (
                "a { position: absolute; position: running( header ) }",
                "a { position: absolute;}",
            ),
            (
                "a{position:running(h)}b{position:running(h);top:0}",
                "a{}b{;top:0}",
            ),
            (
                "h1 { string-set: t content() } p::before { content: 'a' string(t) ; display: block }",
                "h1 { string-set: t content() } p::before {; display: block }",
            ),
            (
                "a::after { content: leader('.') target-counter(attr(href), page) }",
                "a::after {}",
            ),
            (
                "@page { @top-center { content: element(header) } } a { color: red }",
                "@page { @top-center {} } a { color: red }",
            ),
            (
                "@media print { a { b { float: footnote } float: right } }",
                "@media print { a { b {} float: right } }",
            ),
            (
                "a { content: counter(x) \"é\" target-text(url(#a)) }",
                "a {}",
            ),
        ] {
            assert_eq!(unpaged(css), want, "{css}");
        }
    }
}
