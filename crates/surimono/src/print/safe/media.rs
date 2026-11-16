//! Print media in the output (Q-PKG-03).
//!
//! Chrome prints with the media type `print`; a browserless printer lays the
//! page out with `screen` (fulgur 0.40 and 0.41, through stylo's device), so
//! `@media print` rules never apply there and screen-only rules do. This pass
//! rewrites every media query whose answer depends on the type so that it
//! gives, under *any* type, the answer it gives under `print`. Only the type
//! is touched: what a query asks of the page (`min-width`, `orientation`, ...)
//! stays as it was, for the printer to answer against its own page.
//!
//! Each edit replaces tokens of one media query (Media Queries 4 §3, the
//! `<media-query>` grammar) and nothing else, so a rule keeps its place, its
//! block and its layer:
//!
//! | Query (in print) | Becomes | Why it is the same in Chrome's print |
//! |---|---|---|
//! | `print`, `only print`, `print and C`, `only print and C` | `all`, `all and C` | §2.3: `print` matches when printing, as `all` always does; `only` is ignored (§2.4.2) |
//! | `not print`, `not print and C` | `not all`, `not all and C` | the same type swap under `not` (§2.4.1) |
//! | `screen`, `only screen`, `screen and C`, `only screen and C` | `not all` | `screen` doesn't match in print, and false `and` anything is false, valid or not (§2.4.3, §3.2: an invalid query is `not all`) |
//! | `not screen` | `all` | `not` of a type that doesn't match |
//! | `not screen and C` | `not speech and C` | a type swap again: `speech` doesn't match in print or on screen, so the query stays `not (false and C)` and valid exactly when it was |
//!
//! A query that starts with a condition (`(min-width: 600px)`, `not (color)`),
//! names another type (`all`, `tv`, `speech`, `foo`), or doesn't parse into a
//! type at the start is left exactly as it was: the type isn't what decides it
//! in a printer whose type is `screen` or `print`. Types are compared ASCII
//! case-insensitively, as identifiers with their escapes decoded (§2.3); the
//! reserved words `only`, `not`, `and`, `or` and `layer` are no type
//! (§3.1), so a query using one as its type is left alone.
//!
//! Where the queries are:
//!
//! - the prelude of every `@media` rule in a sheet's rule lists, at the top,
//!   inside `@supports`, `@layer`, `@container`, `@scope`, `@starting-style`
//!   and `@document`, and nested in style rules (CSS Nesting 1 §3.1); never
//!   in a declaration's value (a custom property may hold `{@media print{}}`
//!   as tokens) or in a block whose at-rule holds no rules;
//! - the media list at the end of an `@import`'s prelude (CSS Cascade 5
//!   §6.1), after its URL and an optional `layer` and `supports()`;
//! - the `media` attribute of an HTML `<style>` (HTML, "the style element":
//!   a valid media query list, evaluated as `@media` evaluates one). Linked
//!   sheets a loader brought in are `<style media>` by now; a `<link>` left
//!   in the page is CSS that wasn't read, and then nothing is rewritten
//!   (`super`, assumption 3).
//!
//! What this module finds is only a plan: `super` writes a sheet's text only
//! where Chrome can't show it, and an attribute only where nothing reads it.
//! The page's CSS reads the attribute ([`Resolved::reads_media_attribute`],
//! [`reads_media_attribute`]) when `media` is an identifier or string in a
//! square bracket (an attribute selector, `[*|media]` too) or among the
//! arguments of an `attr()` (CSS Values 5 §7.7: `content: attr(media)`, and
//! typed `attr()` in any property), at any depth: in rules nested in
//! `@media` or in style rules, in custom properties' values, in `var()`
//! fallbacks, and in `style` attributes.
//!
//! The reading is token-aware (`cssparser`, CSS Syntax 3: strings, comments,
//! escapes and `url()` are single tokens), one pass over each text, every
//! token a step of the pass's budget, brackets at most [`MAX_NESTING`] deep:
//! anything deeper is not looked at, which only leaves those queries as they
//! were. Every edit stands on its own, so any subset of them is as neutral as
//! all of them.

use std::ops::Range;

use cssparser::{Parser, ParserInput, Token};

use super::{Stop, Work};
use crate::print::MAX_NESTING;

/// One replacement in a text.
type Edit = (Range<usize>, &'static str);

/// The at-rules whose blocks hold rules, where a `@media` may be.
const GROUPS: &[&str] = &[
    "media",
    "supports",
    "layer",
    "container",
    "scope",
    "starting-style",
    "document",
];

/// Words that can't be a media type (Media Queries 4 §3.1, `<media-type>`).
const RESERVED: &[&str] = &["only", "not", "and", "or", "layer"];

/// What resolving a sheet's text found.
#[derive(Debug, Default)]
pub(super) struct Resolved {
    /// The text with its media resolved, if any query changed.
    pub(super) text: Option<String>,
    /// A square bracket in it names `media`: a selector may read the attribute.
    pub(super) reads_media_attribute: bool,
}

/// `css`, the text of a style sheet, with every `@media` and `@import` media
/// list resolved for print.
pub(super) fn resolve_sheet(css: &str, work: &mut Work<'_>) -> Result<Resolved, Stop> {
    work.step(1 + css.len() as u64 / 64)?;
    let mut scan = Scan {
        css,
        edits: Vec::new(),
        reads_media_attribute: false,
        work,
    };
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    scan.list(&mut parser, 0)?;
    let Scan {
        edits,
        reads_media_attribute,
        ..
    } = scan;
    Ok(Resolved {
        text: apply(css, &edits),
        reads_media_attribute,
    })
}

/// Whether `css`, a `style` attribute's declarations, may read a `media`
/// attribute: an `attr()` naming `media`, at any depth, custom properties'
/// values included.
pub(super) fn reads_media_attribute(css: &str, work: &mut Work<'_>) -> Result<bool, Stop> {
    work.step(1 + css.len() as u64 / 64)?;
    let mut scan = Scan {
        css,
        edits: Vec::new(),
        reads_media_attribute: false,
        work,
    };
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    scan.list(&mut parser, 0)?;
    Ok(scan.reads_media_attribute)
}

/// A `media` attribute's value resolved for print, or `None` if it stays.
pub(super) fn resolve_list(value: &str, work: &mut Work<'_>) -> Result<Option<String>, Stop> {
    work.step(1 + value.len() as u64 / 64)?;
    let mut edits = Vec::new();
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    query_list(&mut parser, 0, &mut edits, work)?;
    Ok(apply(value, &edits))
}

/// Whether the media list `value` is `not all` and nothing else, query by
/// query: it never matches.
pub(super) fn never(value: &str) -> bool {
    !value.trim().is_empty()
        && value
            .split(',')
            .all(|q| q.trim().eq_ignore_ascii_case("not all"))
}

/// `text` with `edits` (in order, not overlapping) made, if there are any.
fn apply(text: &str, edits: &[Edit]) -> Option<String> {
    if edits.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 8 * edits.len());
    let mut at = 0;
    for (range, with) in edits {
        out.push_str(&text[at..range.start]);
        out.push_str(with);
        at = range.end;
    }
    out.push_str(&text[at..]);
    Some(out)
}

struct Scan<'a, 'w, 'g> {
    css: &'a str,
    edits: Vec<Edit>,
    reads_media_attribute: bool,
    work: &'w mut Work<'g>,
}

/// What a rule list has shown of the item being read.
#[derive(Default)]
struct Item {
    /// Tokens other than whitespace and comments.
    count: usize,
    /// It starts with this at-keyword, in lower case; its prelude starts here.
    at: Option<(String, usize)>,
    /// It starts with an identifier, in lower case.
    ident: Option<String>,
    /// A colon came second: it is a declaration.
    declaration: bool,
}

impl Item {
    /// A custom property's declaration, which a `{}` doesn't end.
    fn custom(&self) -> bool {
        self.declaration && self.ident.as_deref().is_some_and(|n| n.starts_with("--"))
    }
}

impl Scan<'_, '_, '_> {
    /// A list of rules (and, nested, declarations), told apart as CSS Syntax 3
    /// §5.4 does: an at-keyword starts an at-rule, which ends at a `;` or a
    /// `{}`; anything else ending at a `{}` is a style rule, unless it is a
    /// custom property's value.
    fn list(&mut self, p: &mut Parser<'_, '_>, depth: usize) -> Result<(), Stop> {
        let mut item = Item::default();
        loop {
            let before = p.position().byte_index();
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                break;
            };
            let token = token.clone();
            self.work.step(1)?;
            let after = p.position().byte_index();
            match &token {
                Token::WhiteSpace(_) | Token::Comment(_) => continue,
                Token::CDO | Token::CDC if item.count == 0 => continue,
                Token::Semicolon => {
                    if let Some((name, start)) = &item.at
                        && name == "import"
                    {
                        self.import(start.to_owned()..before)?;
                    }
                    item = Item::default();
                    continue;
                }
                Token::CurlyBracketBlock if !item.custom() => {
                    let descend = match &item.at {
                        Some((name, start)) => {
                            if name == "media" {
                                let prelude = *start..before;
                                let mut input = ParserInput::new(&self.css[prelude.clone()]);
                                let mut q = Parser::new(&mut input);
                                let mut edits = Vec::new();
                                query_list(&mut q, depth, &mut edits, self.work)?;
                                self.edits.extend(edits.into_iter().map(|(r, w)| {
                                    (r.start + prelude.start..r.end + prelude.start, w)
                                }));
                            }
                            GROUPS.contains(&name.as_str())
                        }
                        // A style rule: its block holds declarations and rules.
                        None => true,
                    };
                    if descend {
                        self.nested(p, depth, |s, p| s.list(p, depth + 1))?;
                    }
                    item = Item::default();
                    continue;
                }
                Token::AtKeyword(name) if item.count == 0 => {
                    item.at = Some((name.to_ascii_lowercase(), after));
                }
                Token::Ident(name) if item.count == 0 => {
                    item.ident = Some(name.to_ascii_lowercase());
                }
                Token::Colon if item.count == 1 && item.ident.is_some() => {
                    item.declaration = true;
                }
                Token::SquareBracketBlock => {
                    self.nested(p, depth, |s, p| s.bracket(p, depth + 1))?;
                }
                _ => {}
            }
            item.count += 1;
            if matches!(
                token,
                Token::Function(_) | Token::ParenthesisBlock | Token::CurlyBracketBlock
            ) {
                let attr = is_attr(&token);
                self.nested(p, depth, |s, p| s.inside(p, depth + 1, attr))?;
            }
        }
        // A sheet may end in an `@import` with no `;`.
        if let Some((name, start)) = &item.at
            && name == "import"
        {
            let end = self.css.len().min(p.position().byte_index());
            self.import(*start..end)?;
        }
        Ok(())
    }

    /// Runs `inner` on the block that was just read, unless that is deeper
    /// than [`MAX_NESTING`]; then its content is skipped unread.
    fn nested(
        &mut self,
        p: &mut Parser<'_, '_>,
        depth: usize,
        inner: impl FnOnce(&mut Self, &mut Parser<'_, '_>) -> Result<(), Stop>,
    ) -> Result<(), Stop> {
        if depth >= MAX_NESTING {
            return Ok(());
        }
        let mut stopped = None;
        let read: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
            stopped = inner(self, p).err();
            Ok(())
        });
        drop(read);
        stopped.map_or(Ok(()), Err)
    }

    /// What's in a function or a bracket that isn't a rule's block: only its
    /// square brackets and `attr()` functions matter here. `attr` says it is
    /// an `attr()`'s arguments: one that names `media` reads the attribute.
    fn inside(&mut self, p: &mut Parser<'_, '_>, depth: usize, attr: bool) -> Result<(), Stop> {
        while let Ok(token) = p.next_including_whitespace_and_comments() {
            let token = token.clone();
            self.work.step(1)?;
            match &token {
                Token::Ident(name) | Token::QuotedString(name)
                    if attr && name.eq_ignore_ascii_case("media") =>
                {
                    self.reads_media_attribute = true;
                }
                Token::SquareBracketBlock => {
                    self.nested(p, depth, |s, p| s.bracket(p, depth + 1))?;
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::CurlyBracketBlock => {
                    let attr = attr || is_attr(&token);
                    self.nested(p, depth, |s, p| s.inside(p, depth + 1, attr))?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A square bracket: an attribute selector, perhaps. One that names
    /// `media`, at any depth, may read the attribute.
    fn bracket(&mut self, p: &mut Parser<'_, '_>, depth: usize) -> Result<(), Stop> {
        while let Ok(token) = p.next_including_whitespace_and_comments() {
            let token = token.clone();
            self.work.step(1)?;
            match &token {
                Token::Ident(name) | Token::QuotedString(name)
                    if name.eq_ignore_ascii_case("media") =>
                {
                    self.reads_media_attribute = true;
                }
                Token::SquareBracketBlock => {
                    self.nested(p, depth, |s, p| s.bracket(p, depth + 1))?;
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::CurlyBracketBlock => {
                    self.nested(p, depth, |s, p| s.bracket(p, depth + 1))?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The prelude of an `@import` at `prelude`: a URL, then perhaps `layer`
    /// or `layer(…)`, then perhaps `supports(…)`, then the media list (CSS
    /// Cascade 5 §6.1). Anything else and it stays.
    fn import(&mut self, prelude: Range<usize>) -> Result<(), Stop> {
        let text = &self.css[prelude.clone()];
        let mut input = ParserInput::new(text);
        let mut p = Parser::new(&mut input);
        // 0: the URL is next; 1: `layer` may be; 2: `supports()` may be.
        let mut stage = 0;
        let list_start = loop {
            let state = p.state();
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                return Ok(());
            };
            let token = token.clone();
            self.work.step(1)?;
            match (&token, stage) {
                (Token::WhiteSpace(_) | Token::Comment(_), _) => continue,
                (Token::QuotedString(_) | Token::UnquotedUrl(_), 0) => stage = 1,
                (Token::Function(name), 0) if name.eq_ignore_ascii_case("url") => stage = 1,
                (Token::Ident(name), 1) if name.eq_ignore_ascii_case("layer") => stage = 2,
                (Token::Function(name), 1) if name.eq_ignore_ascii_case("layer") => stage = 2,
                (Token::Function(name), 1 | 2) if name.eq_ignore_ascii_case("supports") => {
                    stage = 3;
                }
                (_, 1..) => {
                    p.reset(&state);
                    break state.position().byte_index();
                }
                _ => return Ok(()),
            }
        };
        let mut edits = Vec::new();
        query_list(&mut p, 0, &mut edits, self.work)?;
        let base = prelude.start;
        debug_assert!(edits.iter().all(|(r, _)| r.start >= list_start));
        self.edits.extend(
            edits
                .into_iter()
                .map(|(r, w)| (r.start + base..r.end + base, w)),
        );
        Ok(())
    }
}

/// Whether `token` opens an `attr()` (CSS Values 5 §7.7), its name ASCII
/// case-insensitive with its escapes decoded, as cssparser gives it.
fn is_attr(token: &Token<'_>) -> bool {
    matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("attr"))
}

/// A token of a media query that isn't whitespace or a comment.
struct Part {
    kind: Kind,
    at: Range<usize>,
}

enum Kind {
    /// An identifier, in lower case.
    Ident(String),
    /// A parenthesis or a function, with all it holds.
    Group,
    Other,
}

impl Part {
    fn is(&self, word: &str) -> bool {
        matches!(&self.kind, Kind::Ident(w) if w == word)
    }
}

/// Reads a media query list from `p` to its end and adds the edits that
/// resolve it for print to `edits`, at offsets in `p`'s input.
fn query_list(
    p: &mut Parser<'_, '_>,
    depth: usize,
    edits: &mut Vec<Edit>,
    work: &mut Work<'_>,
) -> Result<(), Stop> {
    let mut query: Vec<Part> = Vec::new();
    loop {
        let start = p.position().byte_index();
        let token = p.next_including_whitespace_and_comments().cloned();
        work.step(1)?;
        let kind = match &token {
            Err(_) | Ok(Token::Comma) => {
                edits.extend(resolve(&query));
                query.clear();
                if token.is_err() {
                    return Ok(());
                }
                continue;
            }
            Ok(Token::WhiteSpace(_) | Token::Comment(_)) => continue,
            Ok(Token::Ident(name)) => Kind::Ident(name.to_ascii_lowercase()),
            Ok(Token::Function(_) | Token::ParenthesisBlock) => Kind::Group,
            Ok(_) => Kind::Other,
        };
        if matches!(
            token,
            Ok(Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock)
        ) {
            // What's inside isn't read: only the type is resolved, and it is
            // never in a bracket. The tokens still count.
            if depth >= MAX_NESTING {
                return Ok(());
            }
            let mut stopped = None;
            let read: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
                stopped = count(p, depth + 1, work).err();
                Ok(())
            });
            drop(read);
            if let Some(stop) = stopped {
                return Err(stop);
            }
        }
        let end = p.position().byte_index();
        query.push(Part {
            kind,
            at: start..end,
        });
    }
}

/// Counts the tokens of a block, at most [`MAX_NESTING`] deep.
fn count(p: &mut Parser<'_, '_>, depth: usize, work: &mut Work<'_>) -> Result<(), Stop> {
    while let Ok(token) = p.next_including_whitespace_and_comments() {
        let opens = matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        work.step(1)?;
        if opens && depth < MAX_NESTING {
            let mut stopped = None;
            let read: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
                stopped = count(p, depth + 1, work).err();
                Ok(())
            });
            drop(read);
            if let Some(stop) = stopped {
                return Err(stop);
            }
        }
    }
    Ok(())
}

/// The edit that resolves one media query for print; see the module docs.
fn resolve(query: &[Part]) -> Option<Edit> {
    let (first, last) = (query.first()?, query.last()?);
    let (prefix, at) = if first.is("not") || first.is("only") {
        (Some(first), 1)
    } else {
        (None, 0)
    };
    let ty = query.get(at)?;
    let Kind::Ident(name) = &ty.kind else {
        return None;
    };
    if RESERVED.contains(&name.as_str()) {
        return None;
    }
    // After the type comes nothing, or `and` and a condition.
    let rest = &query[at + 1..];
    if rest.first().is_some_and(|p| !p.is("and")) {
        // Not a `<media-query>`: Chrome reads it as `not all` (§3.2), and so
        // will any printer; it is left as it is.
        return None;
    }
    let negated = prefix.is_some_and(|p| p.is("not"));
    let whole = first.at.start..last.at.end;
    match (name.as_str(), negated) {
        ("print", false) => Some((first.at.start..ty.at.end, "all")),
        ("print", true) => Some((ty.at.clone(), "all")),
        ("screen", false) => Some((whole, "not all")),
        ("screen", true) if rest.is_empty() => Some((whole, "all")),
        ("screen", true) => Some((ty.at.clone(), "speech")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use super::super::super::Gate;
    use super::*;

    fn with_work<T>(f: impl FnOnce(&mut Work<'_>) -> T) -> T {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let mut work = Work {
            steps: 0,
            gate: &gate,
        };
        f(&mut work)
    }

    /// The sheet resolved, or as it was.
    fn sheet(css: &str) -> String {
        with_work(|w| match resolve_sheet(css, w) {
            Ok(r) => r.text.unwrap_or_else(|| css.to_owned()),
            Err(_) => panic!("stopped"),
        })
    }

    fn list(value: &str) -> String {
        with_work(|w| match resolve_list(value, w) {
            Ok(r) => r.unwrap_or_else(|| value.to_owned()),
            Err(_) => panic!("stopped"),
        })
    }

    #[test]
    fn a_print_type_becomes_all() {
        // Media Queries 4 §2.3 (`print`, `all`) and §2.4.2 (`only`).
        assert_eq!(list("print"), "all");
        assert_eq!(list("PRINT"), "all");
        assert_eq!(list("only print"), "all");
        assert_eq!(
            list("print and (min-width: 600px)"),
            "all and (min-width: 600px)"
        );
        assert_eq!(
            list("only print and (orientation:portrait) and (color)"),
            "all and (orientation:portrait) and (color)"
        );
        // An escape is the letter it stands for (§2.3, CSS Syntax 3 §4.3.7).
        assert_eq!(list("\\70 rint"), "all");
    }

    #[test]
    fn a_negated_print_type_stays_negated() {
        // §2.4.1: `not` negates the whole query.
        assert_eq!(list("not print"), "not all");
        assert_eq!(
            list("not print and (min-width: 1px)"),
            "not all and (min-width: 1px)"
        );
    }

    #[test]
    fn a_screen_type_never_matches() {
        assert_eq!(list("screen"), "not all");
        assert_eq!(list("only screen"), "not all");
        assert_eq!(list("screen and (max-width: 600px)"), "not all");
        // False `and` anything stays false, whether the condition is valid
        // or not (§3.2).
        assert_eq!(list("screen and (max-width: 600px) or (color)"), "not all");
        assert_eq!(list("not screen"), "all");
        // Under `not` with a condition, a type that matches nowhere keeps the
        // query valid exactly when it was.
        assert_eq!(list("not screen and (color)"), "not speech and (color)");
    }

    #[test]
    fn each_query_of_a_list_on_its_own() {
        assert_eq!(list("screen, print"), "not all, all");
        assert_eq!(
            list("print and (min-width: 1px), (max-width: 2px), tv"),
            "all and (min-width: 1px), (max-width: 2px), tv"
        );
        assert_eq!(
            list(" /* a */ print /* b */ , screen "),
            " /* a */ all /* b */ , not all "
        );
    }

    #[test]
    fn what_the_type_doesnt_decide_stays() {
        for same in [
            "",
            "all",
            "not all",
            "tv",
            "speech",
            "(min-width: 600px)",
            "not (color)",
            "(print)",
            // Reserved words aren't types (§3.1).
            "only",
            "not",
            "layer",
            "only and (color)",
            "not only print",
            // Not a `<media-query>`: Chrome's `not all` as it stands.
            "print (color)",
            "print or (color)",
            "screen screen",
            "print, &",
            "\"print\"",
        ] {
            let resolved = list(same);
            if same == "print, &" {
                assert_eq!(resolved, "all, &");
            } else {
                assert_eq!(resolved, same, "{same:?}");
            }
        }
    }

    #[test]
    fn media_rules_everywhere_rules_are() {
        assert_eq!(sheet("@media print{a{b:c}}"), "@media all{a{b:c}}");
        assert_eq!(sheet("@MEDIA screen { a{} }"), "@MEDIA not all { a{} }");
        assert_eq!(sheet("@m\\65 dia print{}"), "@m\\65 dia all{}");
        assert_eq!(
            sheet("@supports (x:y){@layer l{@media print{@media screen{a{}}}}}"),
            "@supports (x:y){@layer l{@media all{@media not all{a{}}}}}"
        );
        // CSS Nesting 1 §3.1: conditional rules nested in a style rule.
        assert_eq!(
            sheet("p { color: red; @media print { color: blue } }"),
            "p { color: red; @media all { color: blue } }"
        );
        assert_eq!(
            sheet("@container (min-width: 1px) { @media print {} } @scope (a) { @media print {} }"),
            "@container (min-width: 1px) { @media all {} } @scope (a) { @media all {} }"
        );
    }

    #[test]
    fn what_isnt_a_rule_stays() {
        for same in [
            // Values and strings are tokens of their own.
            "a { --x: { @media print {} } }",
            "a { --x: @media print; }",
            "a { content: \"@media print {}\" }",
            "/* @media print {} */",
            "a { background: url(@media print) }",
            // Blocks that hold no rules.
            "@keyframes k { @media print {} }",
            "@font-face { @media print {} }",
            "@page { @media print {} }",
            "@whatever { @media print {} }",
            // A statement, no block: invalid as it is.
            "@media print;",
        ] {
            assert_eq!(sheet(same), same, "{same:?}");
        }
    }

    #[test]
    fn imports_keep_their_url_layer_and_supports() {
        assert_eq!(
            sheet("@import url(a.css) print;"),
            "@import url(a.css) all;"
        );
        assert_eq!(
            sheet("@import 'a.css' screen, print"),
            "@import 'a.css' not all, all"
        );
        assert_eq!(
            sheet("@import url('a.css') layer(x) supports(display: grid) print and (color);"),
            "@import url('a.css') layer(x) supports(display: grid) all and (color);"
        );
        assert_eq!(
            sheet("@import url(a.css) layer screen;"),
            "@import url(a.css) layer not all;"
        );
        for same in [
            "@import url(a.css);",
            "@import print;",
            "@import url(a.css) supports(x) layer print;",
        ] {
            assert_eq!(sheet(same), same, "{same:?}");
        }
    }

    #[test]
    fn a_media_attribute_selector_is_seen() {
        let reads =
            |css: &str| with_work(|w| resolve_sheet(css, w).ok().unwrap().reads_media_attribute);
        assert!(reads("style[media] + p {}"));
        assert!(reads("p:not([MEDIA=print]) {}"));
        assert!(reads("@media print { :is([*|media]) {} }"));
        assert!(reads("[\"media\"] {}"));
        assert!(!reads("[data-media] {} .media {} a { content: 'media' }"));
    }

    #[test]
    fn the_work_is_counted_and_bounded() {
        let deep = format!(
            "{}@media print{{}}{}",
            "@media print{".repeat(64),
            "}".repeat(64)
        );
        let out = sheet(&deep);
        // Every rule whose prelude is read within the nesting bound is
        // resolved, none past it.
        assert_eq!(out.matches("@media all{").count(), MAX_NESTING + 1);
        let long = "@media print{}".repeat(100_000);
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let mut work = Work {
            steps: (1 << 24) - 1000,
            gate: &gate,
        };
        assert!(matches!(resolve_sheet(&long, &mut work), Err(Stop::Budget)));
    }

    #[test]
    fn html_comment_tokens_around_a_sheet_are_skipped() {
        // CSS Syntax 3 §5.4.1: `<!--` and `-->` at the top level are dropped.
        assert_eq!(
            sheet("<!-- @media print { a {} } -->"),
            "<!-- @media all { a {} } -->"
        );
        assert_eq!(
            sheet("<!--\n@import url(a.css) screen;\n-->"),
            "<!--\n@import url(a.css) not all;\n-->"
        );
    }

    #[test]
    fn a_query_past_the_nesting_bound_stays() {
        // Defence only: a page can't get here, the screen refuses CSS nested
        // deeper than `MAX_NESTING` first. The query at the bound isn't read.
        let open = "@media print{".repeat(MAX_NESTING);
        let close = "}".repeat(MAX_NESTING);
        let deep = format!("{open}@media print and (color){{}}{close}");
        let out = sheet(&deep);
        assert_eq!(out.matches("@media all{").count(), MAX_NESTING);
        assert!(out.contains("@media print and (color){}"), "{out}");
    }

    #[test]
    fn attr_of_media_is_seen() {
        let reads =
            |css: &str| with_work(|w| resolve_sheet(css, w).ok().unwrap().reads_media_attribute);
        assert!(reads("style::before { content: attr(media) }"));
        assert!(reads("p { --x: attr( MEDIA type(<length>)) }"));
        assert!(reads(
            "@media print { p { & b { x: var(--y, calc(attr(media) + 1)) } } }"
        ));
        assert!(!reads(
            "p { content: attr(data-media) } q { content: 'attr(media)' }"
        ));
        let in_style = |css: &str| with_work(|w| reads_media_attribute(css, w).ok().unwrap());
        assert!(in_style("--m: attr(media)"));
        assert!(!in_style("color: red; --media: 1"));
    }
}
