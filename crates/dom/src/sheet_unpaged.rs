//! Style text as a browser keeps it of what only a paged medium has, for the
//! Raikiri engine's measuring copy (`layout_raikiri.rs`).
//!
//! Raikiri is a printer's layout: it knows CSS Generated Content for Paged
//! Media, which no browser does. The measuring copy gets the text a browser
//! would keep, so that a script measures the page laid out as in Chromium
//! 153 (tests/data/chrome-layout: `paged-only`, `paged-var`, `paged-rules`,
//! `paged-supports`, each Chromium's):
//!
//! - `float: footnote`, `position: running(<name>)`, and a `content` with
//!   `string()`, `element()`, `leader()`, `target-counter()`,
//!   `target-counters()` or `target-text()` in it, are unknown values: the
//!   declaration is dropped as it is parsed, so whatever an earlier
//!   declaration said still holds. Such a declaration is cut out.
//! - The same values in a declaration that holds `var()` (or a custom
//!   property that holds them, which a `var()` can bring in): the browser
//!   keeps the declaration, and once `var()` is substituted it is invalid at
//!   computed-value time, so the property has its initial value
//!   (`float: none`, `position: static`, `content: normal`). Such a value is
//!   renamed (`-surimono-footnote`, `-surimono-string(`), which Raikiri
//!   doesn't know either, and Raikiri, which keeps a declaration with
//!   `var()` until it substitutes, then does the same.
//! - A `content` with `attr()` and such a function: kept by the browser as
//!   for `var()`, and invalid once `attr()` is substituted; Raikiri reads
//!   `attr()` as it parses, so the declaration becomes `content: normal`.
//! - An `@supports` condition on any of them, or on `string-set` or the
//!   `bookmark-*` properties, is false in the browser: they are renamed in
//!   the condition, so that it is false here too.
//!
//! Only declaration blocks are read for declarations (style rules, `@page`
//! and its margin boxes, `@font-face` and their like, and group rules inside
//! a style rule): at the top level and in the block of `@media`,
//! `@supports`, `@layer`, `@container` and their like a browser reads
//! `float: footnote; a { }` as one rule whose selector is invalid, and the
//! text there is left as it is. The text is read as tokens, never as words,
//! so a string or a comment that spells one of them is left alone, and an
//! escape (`f\6fotnote`) is read as the tokenizer reads it.
//!
//! **Bounds.** The text was bounded first by the sheet screen
//! (`sheet_screen.rs`) or `style_attr::parse`: nesting to
//! [`MAX_NESTING`], which the reading here relies on and checks again, and
//! bytes to the engine's sheet and attribute bounds. Each token is read
//! once, and the deadline is looked at every [`POLL_TOKENS`] tokens.

use std::borrow::Cow;
use std::ops::Range;

use cssparser::{ParseError, Parser, ParserInput, Token};
use surimono_core::Deadline;

use crate::sheet_screen::MAX_NESTING;

/// What the text has to spell, in any case, for it to be read at all:
/// looked for first, so that almost every text is not read twice. A
/// backslash, since an escape can spell any of them.
const NEEDLES: [&[u8]; 9] = [
    b"footnote",
    b"string-set",
    b"running(",
    b"string(",
    b"element(",
    b"leader(",
    b"target-",
    b"bookmark-",
    b"\\",
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

/// Properties a browser doesn't know, for `@supports`.
const PAGED_PROPERTIES: [&str; 4] = [
    "string-set",
    "bookmark-label",
    "bookmark-level",
    "bookmark-state",
];

/// What a renamed value begins with: no engine knows it.
const RENAMED: &str = "-surimono-";

/// Tokens between looks at the deadline.
const POLL_TOKENS: usize = 4_096;

/// The page's deadline passed while the text was read.
#[derive(Debug, PartialEq)]
pub(crate) struct TimeLimit;

/// A sheet's text as a browser keeps it of what only a paged medium has.
/// Borrowed when nothing changes, which is almost always.
pub(crate) fn unpaged<'a>(
    css: &'a str,
    deadline: Option<&Deadline>,
) -> Result<Cow<'a, str>, TimeLimit> {
    rewrite(css, Block::Rules, deadline)
}

/// [`unpaged`] for the declarations of a `style` attribute.
pub(crate) fn unpaged_attribute<'a>(
    css: &'a str,
    deadline: Option<&Deadline>,
) -> Result<Cow<'a, str>, TimeLimit> {
    rewrite(css, Block::Declarations, deadline)
}

fn rewrite<'a>(
    css: &'a str,
    block: Block,
    deadline: Option<&Deadline>,
) -> Result<Cow<'a, str>, TimeLimit> {
    let bytes = css.as_bytes();
    let spelled = |needle: &[u8]| {
        bytes
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    };
    if !NEEDLES.iter().any(|needle| spelled(needle)) {
        return Ok(Cow::Borrowed(css));
    }
    let mut reader = Reader {
        edits: Vec::new(),
        tokens: 0,
        deadline,
        late: false,
    };
    let mut input = ParserInput::new(css);
    reader.walk(&mut Parser::new(&mut input), block, 0);
    if reader.late {
        return Err(TimeLimit);
    }
    if reader.edits.is_empty() {
        return Ok(Cow::Borrowed(css));
    }
    reader.edits.sort_by_key(|e| (e.range.start, e.range.end));
    let mut out = String::with_capacity(css.len() + reader.edits.len() * RENAMED.len());
    let mut from = 0;
    for edit in reader.edits {
        // Edits never overlap; a boundary that isn't a character's is left
        // alone (the tokenizer gives none).
        if edit.range.start < from
            || css.get(from..edit.range.start).is_none()
            || css.get(edit.range.end..).is_none()
        {
            continue;
        }
        out.push_str(&css[from..edit.range.start]);
        out.push_str(edit.with);
        from = edit.range.end;
    }
    out.push_str(&css[from..]);
    Ok(Cow::Owned(out))
}

/// One change to the text: `range` becomes `with`.
struct Edit {
    range: Range<usize>,
    with: &'static str,
}

/// What a `{}` block holds.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    /// Rules only: the top level of a sheet, and `@media` and its like there.
    Rules,
    /// Declarations, and rules nested among them.
    Declarations,
}

#[derive(Clone, Copy, PartialEq)]
enum Property {
    Float,
    Position,
    Content,
    /// A custom property, `--*`.
    Custom,
}

/// What has been read of one statement: a declaration, until a `{}` shows it
/// was a rule, or an at-rule's prelude.
#[derive(Default)]
struct Statement {
    /// Tokens read at its own level, without white space and comments.
    tokens: usize,
    /// The at-rule it begins with, in ASCII lower case, if it does.
    at_rule: Option<String>,
    property: Option<Property>,
    colon: bool,
    /// Value tokens read, a function or a bracket with all it holds as one,
    /// `!important` not counted.
    values: usize,
    /// The value's first token is the one the property is dropped for.
    first_is_paged: bool,
    /// A function a browser doesn't know in `content` is in the value.
    paged_function: bool,
    /// `var()` is in the value.
    var: bool,
    /// `attr()` is in the value.
    attr: bool,
    /// The last value token was `!`.
    bang: bool,
    important: bool,
    /// Where a value a browser doesn't know begins, to be renamed if the
    /// declaration is kept.
    renames: Vec<usize>,
    /// A custom property's value is this one `footnote` alone.
    sole_footnote: Option<usize>,
}

impl Statement {
    /// Reads a token of the statement's own level, beginning at `at`.
    fn read(&mut self, token: &Token<'_>, block: Block, at: usize) {
        self.tokens += 1;
        match (self.tokens, token) {
            (1, Token::AtKeyword(name)) => self.at_rule = Some(name.to_ascii_lowercase()),
            (1, Token::Ident(name)) if block == Block::Declarations => {
                self.property = if name.starts_with("--") {
                    Some(Property::Custom)
                } else {
                    [
                        ("float", Property::Float),
                        ("position", Property::Position),
                        ("content", Property::Content),
                    ]
                    .into_iter()
                    .find_map(|(n, p)| name.eq_ignore_ascii_case(n).then_some(p))
                };
            }
            (2, Token::Colon) if self.property.is_some() => self.colon = true,
            _ if self.colon => self.value(token, at),
            _ => self.in_prelude(token, at),
        }
    }

    fn value(&mut self, token: &Token<'_>, at: usize) {
        if self.bang {
            self.bang = false;
            if matches!(token, Token::Ident(word) if word.eq_ignore_ascii_case("important")) {
                self.important = true;
                return;
            }
            // `!` and something else: a value of two more tokens.
            self.values += 1;
        }
        if matches!(token, Token::Delim('!')) {
            self.bang = true;
            return;
        }
        self.important = false;
        self.values += 1;
        match (self.property, token) {
            (Some(Property::Float), Token::Ident(word)) if self.values == 1 => {
                self.first_is_paged = word.eq_ignore_ascii_case("footnote");
            }
            (Some(Property::Position), Token::Function(name)) if self.values == 1 => {
                self.first_is_paged = name.eq_ignore_ascii_case("running");
            }
            (Some(Property::Custom), Token::Ident(word))
                if self.values == 1 && word.eq_ignore_ascii_case("footnote") =>
            {
                self.sole_footnote = Some(at);
            }
            _ => {}
        }
        self.inner(token, at);
    }

    /// A token of an at-rule's prelude, at any depth.
    fn in_prelude(&mut self, token: &Token<'_>, at: usize) {
        if self.at_rule.as_deref() != Some("supports") {
            return;
        }
        let paged = match token {
            Token::Ident(word) => {
                word.eq_ignore_ascii_case("footnote")
                    || PAGED_PROPERTIES
                        .iter()
                        .any(|p| word.eq_ignore_ascii_case(p))
            }
            Token::Function(name) => {
                is_paged_function(name) || name.eq_ignore_ascii_case("running")
            }
            _ => false,
        };
        if paged {
            self.renames.push(at);
        }
    }

    /// A token of the value, at any depth: notes what it is and where a value
    /// to be renamed begins.
    fn inner(&mut self, token: &Token<'_>, at: usize) {
        let Token::Function(name) = token else {
            if let (Some(Property::Float), Token::Ident(word)) = (self.property, token)
                && word.eq_ignore_ascii_case("footnote")
            {
                self.renames.push(at);
            }
            return;
        };
        self.var |= name.eq_ignore_ascii_case("var");
        self.attr |= name.eq_ignore_ascii_case("attr");
        let rename = match self.property {
            Some(Property::Position) => name.eq_ignore_ascii_case("running"),
            Some(Property::Content) => {
                let paged = is_paged_function(name);
                self.paged_function |= paged;
                paged
            }
            Some(Property::Custom) => {
                is_paged_function(name) || name.eq_ignore_ascii_case("running")
            }
            _ => false,
        };
        if rename {
            self.renames.push(at);
        }
    }

    /// Whether this is a declaration a browser drops as it parses it.
    fn is_dropped(&self) -> bool {
        self.colon
            && !self.var
            && match self.property {
                Some(Property::Float | Property::Position) => {
                    self.first_is_paged && self.values == 1
                }
                Some(Property::Content) => self.paged_function && !self.attr,
                _ => false,
            }
    }

    /// The edits for this statement, which ran over `range`, as a
    /// declaration.
    fn finish(self, range: Range<usize>, edits: &mut Vec<Edit>) {
        if !self.colon {
            return;
        }
        let rename = |edits: &mut Vec<Edit>, at: usize| {
            edits.push(Edit {
                range: at..at,
                with: RENAMED,
            });
        };
        match self.property {
            Some(Property::Custom) => {
                let sole = self.sole_footnote.filter(|_| self.values == 1);
                for at in self.renames.into_iter().chain(sole) {
                    rename(edits, at);
                }
            }
            Some(_) if self.var => {
                for at in self.renames {
                    rename(edits, at);
                }
            }
            Some(Property::Content) if self.attr && self.paged_function => edits.push(Edit {
                range,
                with: if self.important {
                    "content: normal !important"
                } else {
                    "content: normal"
                },
            }),
            _ if self.is_dropped() => edits.push(Edit { range, with: "" }),
            _ => {}
        }
    }

    /// The edits for an at-rule's prelude, read to its block or its end.
    fn finish_prelude(self, edits: &mut Vec<Edit>) {
        if self.at_rule.is_some() {
            for at in self.renames {
                edits.push(Edit {
                    range: at..at,
                    with: RENAMED,
                });
            }
        }
    }
}

fn is_paged_function(name: &str) -> bool {
    CONTENT_FUNCTIONS
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
}

/// What a block of an at-rule holds, inside a block that holds `outer`.
fn at_rule_block(name: &str, outer: Block) -> Block {
    match name {
        // Group rules: as the block they are in.
        "media" | "supports" | "layer" | "container" | "scope" | "starting-style" | "document"
        | "-moz-document" => outer,
        "page"
        | "font-face"
        | "counter-style"
        | "property"
        | "font-palette-values"
        | "view-transition"
        | "position-try" => Block::Declarations,
        // `@page`'s margin boxes.
        margin
            if ["top-", "bottom-", "left-", "right-"]
                .iter()
                .any(|side| margin.starts_with(side)) =>
        {
            Block::Declarations
        }
        // `@keyframes` (its keyframes are rules) and what isn't known.
        _ => Block::Rules,
    }
}

struct Reader<'d> {
    edits: Vec<Edit>,
    tokens: usize,
    deadline: Option<&'d Deadline>,
    /// The deadline passed: stop.
    late: bool,
}

impl Reader<'_> {
    /// The next token with where it begins, white space and comments
    /// skipped; `None` at the end. Looks at the deadline.
    fn next<'i>(&mut self, p: &mut Parser<'i, '_>) -> Option<(Token<'i>, usize)> {
        loop {
            self.tokens += 1;
            if self.tokens.is_multiple_of(POLL_TOKENS)
                && self.deadline.is_some_and(Deadline::passed)
            {
                self.late = true;
            }
            if self.late {
                return None;
            }
            let at = p.position().byte_index();
            let token = p.next_including_whitespace_and_comments().ok()?.clone();
            if !matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
                return Some((token, at));
            }
        }
    }

    /// Reads the statements of one block (or of the whole text) of kind
    /// `block`, nested `depth` deep.
    fn walk(&mut self, p: &mut Parser<'_, '_>, block: Block, depth: usize) {
        let mut start = p.position().byte_index();
        let mut statement = Statement::default();
        while let Some((token, at)) = self.next(p) {
            match &token {
                Token::Semicolon if block == Block::Declarations || statement.at_rule.is_some() => {
                    let done = std::mem::take(&mut statement);
                    done.finish_prelude_or(start..at, &mut self.edits);
                    start = p.position().byte_index();
                }
                Token::CurlyBracketBlock => {
                    // A rule: its block holds statements of its own.
                    let inner = match &statement.at_rule {
                        Some(name) => at_rule_block(name, block),
                        None => Block::Declarations,
                    };
                    std::mem::take(&mut statement).finish_prelude(&mut self.edits);
                    self.nested(p, depth, |r, p| r.walk(p, inner, depth + 1));
                    start = p.position().byte_index();
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                    statement.read(&token, block, at);
                    self.nested(p, depth, |r, p| r.scan(p, &mut statement, depth + 1));
                }
                _ => statement.read(&token, block, at),
            }
        }
        let end = p.position().byte_index();
        if block == Block::Declarations {
            statement.finish_prelude_or(start..end, &mut self.edits);
        }
    }

    /// Reads to the end of a function's or a bracket's contents, nested
    /// `depth` deep, as part of `statement`.
    fn scan(&mut self, p: &mut Parser<'_, '_>, statement: &mut Statement, depth: usize) {
        while let Some((token, at)) = self.next(p) {
            if statement.colon {
                statement.inner(&token, at);
            } else {
                statement.in_prelude(&token, at);
            }
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) {
                self.nested(p, depth, |r, p| r.scan(p, statement, depth + 1));
            }
        }
    }

    /// Reads the block that was just opened, nested `depth` deep. The
    /// callers bounded nesting to [`MAX_NESTING`] first; were one to stop
    /// doing so, what is deeper is left as it is rather than read.
    fn nested(
        &mut self,
        p: &mut Parser<'_, '_>,
        depth: usize,
        read: impl FnOnce(&mut Self, &mut Parser<'_, '_>),
    ) {
        debug_assert!(
            depth <= MAX_NESTING,
            "the sheet screen and style_attr bound nesting before this"
        );
        if depth > MAX_NESTING {
            return;
        }
        let result: Result<(), ParseError<'_, ()>> = p.parse_nested_block(|p| {
            read(self, p);
            Ok(())
        });
        drop(result);
    }
}

impl Statement {
    /// [`Statement::finish_prelude`] for an at-rule, [`Statement::finish`]
    /// for a declaration.
    fn finish_prelude_or(self, range: Range<usize>, edits: &mut Vec<Edit>) {
        if self.at_rule.is_some() {
            self.finish_prelude(edits);
        } else {
            self.finish(range, edits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(css: &str) -> Cow<'_, str> {
        unpaged(css, None).unwrap()
    }

    fn attribute(css: &str) -> Cow<'_, str> {
        unpaged_attribute(css, None).unwrap()
    }

    #[test]
    fn text_without_paged_values_is_borrowed() {
        for css in [
            "",
            "a { float: left; position: absolute; content: 'x' }",
            // Spelled, but not declared.
            "a { content: 'float: footnote' } /* position: running(x) */",
            ".footnote { color: red } #running(x) { }",
            "a { --note: footnote left; float: var(--note) }",
            "a { float: footnote left; position: running(x) y }",
            "a { background: element(#x) }",
            "a { counter-reset: footnote }",
            // At the top level, and in `@media` there: one invalid rule.
            "float: footnote; a { color: red }",
            "@media screen { float: footnote; a { color: red } }",
        ] {
            assert!(matches!(sheet(css), Cow::Borrowed(_)), "{css}");
        }
    }

    #[test]
    fn paged_declarations_are_taken_out_and_the_rest_is_kept() {
        for (css, want) in [
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
                "a::after { content: leader('.') target-counter(url(#a), page) }",
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
                "a { @media print { float: footnote } }",
                "a { @media print {} }",
            ),
            (
                "a { content: counter(x) \"é\" target-text(url(#a)) }",
                "a {}",
            ),
            // An escape is read as the tokenizer reads it.
            ("a { float: f\\6fotnote }", "a {}"),
            ("a { fl\\6f at: footnote }", "a {}"),
        ] {
            assert_eq!(sheet(css), want, "{css}");
        }
        for (css, want) in [
            ("float: footnote", ""),
            (
                "float:left;FLOAT : FootNote !important;color:red",
                "float:left;;color:red",
            ),
        ] {
            assert_eq!(attribute(css), want, "{css}");
        }
    }

    #[test]
    fn substituted_paged_values_are_renamed_and_attr_content_is_normal() {
        for (css, want) in [
            (
                "a { --f: footnote; float: var(--f) }",
                "a { --f: -surimono-footnote; float: var(--f) }",
            ),
            (
                "a { float: var(--x, f\\6fotnote) }",
                "a { float: var(--x, -surimono-f\\6fotnote) }",
            ),
            (
                "a { position: running(var(--n)) }",
                "a { position: -surimono-running(var(--n)) }",
            ),
            (
                "a { content: var(--k, string(t)) }",
                "a { content: var(--k, -surimono-string(t)) }",
            ),
            (
                "a { --l: leader('.'); content: 'a' var(--l) }",
                "a { --l: -surimono-leader('.'); content: 'a' var(--l) }",
            ),
            (
                "a::after { content: '>'; content: target-counter(attr(href), page) }",
                "a::after { content: '>';content: normal}",
            ),
            (
                "a::after { content: target-counter(attr(href), page) !important }",
                "a::after {content: normal !important}",
            ),
        ] {
            assert_eq!(sheet(css), want, "{css}");
        }
    }

    #[test]
    fn supports_conditions_on_paged_values_are_false() {
        for (css, want) in [
            (
                "@supports (float: footnote) { a { color: red } }",
                "@supports (float: -surimono-footnote) { a { color: red } }",
            ),
            (
                "@supports not (position: running(x)) { a { color: red } }",
                "@supports not (position: -surimono-running(x)) { a { color: red } }",
            ),
            (
                "@supports (string-set: t content()) { }",
                "@supports (-surimono-string-set: t content()) { }",
            ),
        ] {
            assert_eq!(sheet(css), want, "{css}");
        }
    }

    #[test]
    fn a_passed_deadline_stops_the_reading() {
        // A dropped sandbox's deadline counts as passed.
        let deadline = surimono_core::Sandbox::new(surimono_core::Limits::default())
            .unwrap()
            .deadline();
        let css = "a { float: footnote } ".repeat(POLL_TOKENS);
        assert_eq!(unpaged(&css, Some(&deadline)), Err(TimeLimit));
    }
}
