//! The last step of a page: what a printer may safely be given.
//!
//! The tree is clean by the time it is serialized, but the text isn't: the
//! serializer writes comments and raw-text elements unescaped, as the standard
//! says, so a comment holding `--><script>…` or a `<style>` holding
//! `</style><noscript>…` becomes markup in whoever parses the result. So this
//! reads the output as a printer's parser does, and what it finds is what the
//! printer would find:
//!
//! - **Scripts and `<noscript>`:** removed here too, as they are from the tree
//!   (`cleanup.js`). Island markers stay, and [`Options::keep_scripts`] keeps
//!   scripts.
//!
//! A printer's CSS engine isn't under any limit of ours. fulgur hands the page
//! to blitz-dom and stylo, whose parse can't be interrupted, in the caller's
//! process, after surimono has returned. Scripts can write whatever they like
//! into `<style>` and `style` attributes, so everything that reaches stylo goes
//! through here first, with one budget for the whole page: `crates/dom` bounds
//! a single attribute, and many of those add up.
//!
//! - **Size:** [`MAX_CSS_BYTES`] over all style text.
//! - **Declarations:** a push of a property a block already has walks the
//!   block, and `all` pushes every longhand, so a block costs its size squared,
//!   and `all` multiplies it; 60 KB took 11 s (`crates/dom/DESIGN.md`).
//!   [`MAX_BLOCK_DECLARATIONS`] per block, [`MAX_STEPS`] over the page.
//! - **Grid strings:** `grid-template-areas` tokenizes its strings again and
//!   interns each name, and `var()` can carry such a string out of a custom
//!   property, so strings count in both. [`MAX_CELLS`] per declaration,
//!   [`MAX_PAGE_CELLS`] over the page.
//! - **Nesting:** [`MAX_NESTING`] brackets, for the recursion in stylo.
//!
//! A page over any of those fails with
//! [`Error::PageDomLimit`](crate::Error::PageDomLimit): there is nothing
//! sensible to print from half a stylesheet.
//!
//! - **URLs:** blitz-dom panics on a URL it can't resolve in `<img src>`,
//!   `<link href>` and CSS (`docs/upstream/blitz/unresolvable-urls-panic.md`).
//!   Such a URL can't load anything in a browser either, so it goes: the
//!   attribute is removed, and in CSS it becomes `data:,`.
//!
//! The result then goes through the parser again in `page.rs`, under the
//! document's limits, since text that became markup never met them.
//!
//! What isn't bounded here: matching selectors against the tree, which
//! happens in the printer and depends on both. Run your printer under a
//! timeout of its own if pages come from people you don't trust.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use cssparser::{Parser, ParserInput, Token};
use lol_html::errors::RewritingError;
use lol_html::html_content::{ContentType, Element, TextChunk};
use lol_html::{RewriteStrSettings, element, rewrite_str, text};
use url::Url;

use crate::charref::decode_attribute;
use crate::error::Error;
use crate::prepass::is_island;

/// Most bytes of style text in a page: `<style>` and `style` attributes.
/// A large framework's stylesheet is a few MiB.
const MAX_CSS_BYTES: usize = 32 * 1024 * 1024;
/// Most declarations in one block, as `crates/dom` has for an attribute.
const MAX_BLOCK_DECLARATIONS: usize = 4_096;
/// What a page's blocks may cost: `crates/dom`'s bound for one attribute,
/// which took stylo under a few hundred milliseconds, for the whole page.
const MAX_STEPS: u64 = 1 << 26;
/// Longhands one `all` pushes, rounded up.
const ALL_LONGHANDS: u64 = 512;
/// Most words in the strings of one declaration: a 64 by 64 grid.
const MAX_CELLS: usize = 4_096;
/// Most words in strings of grid and custom properties in a page.
const MAX_PAGE_CELLS: usize = 65_536;
/// Deepest nesting of brackets, as `crates/dom` has for an attribute.
const MAX_NESTING: usize = 32;

/// What's left of the page's budget.
#[derive(Debug, Default)]
struct Budget {
    bytes: usize,
    steps: u64,
    cells: usize,
    over: Option<&'static str>,
}

impl Budget {
    fn over(&mut self, what: &'static str) {
        self.over.get_or_insert(what);
    }

    fn charge_block(&mut self, declarations: u64, alls: u64) {
        let cost = declarations
            .saturating_mul(declarations.saturating_add(alls.saturating_mul(ALL_LONGHANDS)));
        self.steps = self.steps.saturating_add(cost);
        if self.steps > MAX_STEPS {
            self.over("too many repeated declarations");
        }
    }
}

/// One `{}` block's count, or the attribute that acts as one.
#[derive(Debug, Default)]
struct Block {
    declarations: u64,
    alls: u64,
    /// At the start of a declaration, where its name is.
    at_name: bool,
    /// The declaration's name is `grid…` or a custom property's.
    counts_cells: bool,
    cells: usize,
}

impl Block {
    fn new() -> Self {
        Self {
            at_name: true,
            ..Self::default()
        }
    }
}

/// The URL as it would be resolved, if it can be.
fn resolves(url: &str) -> bool {
    Url::parse("file:///").is_ok_and(|base| base.join(url).is_ok())
}

/// Reads tokens up to the end of this parser's block.
struct Screen<'b> {
    budget: &'b mut Budget,
    /// Byte ranges of `input` to replace, with their replacements.
    fixes: Vec<(Range<usize>, &'static str)>,
}

impl Screen<'_> {
    fn walk(&mut self, p: &mut Parser<'_, '_>, block: &mut Block, depth: usize) {
        // Whether the next string is an `@import`'s URL, or a `url(`'s.
        let mut url_next = false;
        while self.budget.over.is_none() {
            let start = p.position().byte_index();
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                return;
            };
            let token = token.clone();
            let end = p.position().byte_index();
            match &token {
                Token::WhiteSpace(_) | Token::Comment(_) => continue,
                Token::Semicolon => {
                    block.declarations += 1;
                    block.at_name = true;
                    block.counts_cells = false;
                    block.cells = 0;
                    url_next = false;
                    continue;
                }
                Token::Ident(name) if block.at_name => {
                    block.at_name = false;
                    block.alls += u64::from(name.eq_ignore_ascii_case("all"));
                    block.counts_cells = name.starts_with("--")
                        || name
                            .get(..4)
                            .is_some_and(|n| n.eq_ignore_ascii_case("grid"));
                    continue;
                }
                Token::AtKeyword(name) => {
                    block.at_name = false;
                    url_next = name.eq_ignore_ascii_case("import");
                    continue;
                }
                Token::UnquotedUrl(url) if !resolves(url) => {
                    self.fixes.push((start..end, "url(data:,)"));
                }
                Token::QuotedString(s) => {
                    if url_next && !resolves(s) {
                        self.fixes.push((start..end, "\"data:,\""));
                    }
                    if block.counts_cells {
                        self.cells(block, s);
                    }
                }
                _ => {}
            }
            block.at_name = false;
            let is_url = matches!(&token, Token::Function(f) if f.eq_ignore_ascii_case("url"));
            url_next = false;
            match token {
                Token::CurlyBracketBlock => {
                    let mut inner = Block::new();
                    self.nested(p, &mut inner, depth, false);
                    self.close(&inner);
                    block.at_name = true;
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                    self.nested(p, block, depth, is_url)
                }
                _ => {}
            }
        }
    }

    fn nested(&mut self, p: &mut Parser<'_, '_>, block: &mut Block, depth: usize, url: bool) {
        if depth >= MAX_NESTING {
            self.budget.over("nested too deeply");
            return;
        }
        let result: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
            if url {
                self.first_string_is_url(p);
            }
            self.walk(p, block, depth + 1);
            Ok(())
        });
        drop(result);
    }

    /// `url("…")`: its string, if that comes first.
    fn first_string_is_url(&mut self, p: &mut Parser<'_, '_>) {
        let state = p.state();
        let start = p.position().byte_index();
        if let Ok(Token::QuotedString(s)) = p.next() {
            if !resolves(s) {
                self.fixes
                    .push((start..p.position().byte_index(), "\"data:,\""));
            }
            return;
        }
        p.reset(&state);
    }

    fn cells(&mut self, block: &mut Block, text: &str) {
        let cells = text.split_ascii_whitespace().count();
        block.cells += cells;
        self.budget.cells += cells;
        if block.cells > MAX_CELLS || self.budget.cells > MAX_PAGE_CELLS {
            self.budget.over("too many grid cells");
        }
    }

    fn close(&mut self, block: &Block) {
        let declarations = block.declarations + 1;
        if declarations > MAX_BLOCK_DECLARATIONS as u64 {
            self.budget.over("too many declarations in a block");
        }
        self.budget.charge_block(declarations, block.alls);
    }
}

/// `css` with what mustn't reach a printer replaced, charged to `budget`.
/// `attribute` for the text of a `style` attribute, which is one block.
fn screen_css(css: &str, attribute: bool, budget: &mut Budget) -> Option<String> {
    budget.bytes = budget.bytes.saturating_add(css.len());
    if budget.bytes > MAX_CSS_BYTES {
        budget.over("too much CSS");
        return None;
    }
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut screen = Screen {
        budget,
        fixes: Vec::new(),
    };
    let mut top = Block::new();
    top.at_name = attribute;
    screen.walk(&mut parser, &mut top, 0);
    if attribute {
        screen.close(&top);
    }
    let fixes = screen.fixes;
    if fixes.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(css.len());
    let mut at = 0;
    for (range, with) in fixes {
        out.push_str(&css[at..range.start]);
        out.push_str(with);
        at = range.end;
    }
    out.push_str(&css[at..]);
    Some(out)
}

/// A page over budget, as a handler error (stops the rewrite at once).
#[derive(Debug)]
struct Over(&'static str);

impl std::fmt::Display for Over {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Over {}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn stop_if_over(budget: &Budget) -> HandlerResult {
    match budget.over {
        Some(what) => Err(Box::new(Over(what))),
        None => Ok(()),
    }
}

/// Screens a serialized page; see the module docs. `keep_scripts` is
/// [`Options::keep_scripts`](crate::Options::keep_scripts).
pub(crate) fn screen_page(html: &str, keep_scripts: bool) -> Result<String, Error> {
    let budget = RefCell::new(Budget::default());
    let sheet = RefCell::new(String::new());
    let on_style_text = |t: &mut TextChunk<'_>| -> HandlerResult {
        let mut sheet = sheet.borrow_mut();
        sheet.push_str(t.as_str());
        t.remove();
        if t.last_in_text_node() {
            let mut budget = budget.borrow_mut();
            let fixed = screen_css(&sheet, false, &mut budget);
            stop_if_over(&budget)?;
            t.replace(fixed.as_deref().unwrap_or(&sheet), ContentType::Html);
            sheet.clear();
        }
        Ok(())
    };
    let on_style_attribute = |el: &mut Element<'_, '_>| -> HandlerResult {
        let Some(raw) = el.get_attribute("style") else {
            return Ok(());
        };
        let css = decode_attribute(&raw);
        let mut budget = budget.borrow_mut();
        let fixed = screen_css(&css, true, &mut budget);
        stop_if_over(&budget)?;
        if let Some(fixed) = fixed {
            el.set_attribute("style", &fixed.replace('&', "&amp;"))?;
        }
        Ok(())
    };
    let drop_bad_url = |attribute: &'static str| {
        move |el: &mut Element<'_, '_>| -> HandlerResult {
            let bad = el
                .get_attribute(attribute)
                .is_some_and(|raw| !resolves(&decode_attribute(&raw)));
            if bad {
                el.remove_attribute(attribute);
            }
            Ok(())
        }
    };
    // A template's contents are inert for any parser, and stay as they were.
    let in_template = Rc::new(Cell::new(0_usize));
    let on_template = {
        let in_template = Rc::clone(&in_template);
        move |el: &mut Element<'_, '_>| -> HandlerResult {
            in_template.set(in_template.get() + 1);
            let left = Rc::clone(&in_template);
            if let Some(handlers) = el.end_tag_handlers() {
                handlers.push(Box::new(move |_end| {
                    left.set(left.get().saturating_sub(1));
                    Ok(())
                }));
            }
            Ok(())
        }
    };
    let script_depth = Rc::clone(&in_template);
    let on_script = move |el: &mut Element<'_, '_>| -> HandlerResult {
        if script_depth.get() == 0 && !keep_scripts && !is_island(el) {
            el.remove();
        }
        Ok(())
    };
    let noscript_depth = Rc::clone(&in_template);
    let on_noscript = move |el: &mut Element<'_, '_>| -> HandlerResult {
        if noscript_depth.get() == 0 {
            el.remove();
        }
        Ok(())
    };
    let settings = RewriteStrSettings {
        element_content_handlers: vec![
            element!("template", on_template),
            element!("script", on_script),
            element!("noscript", on_noscript),
            text!("style", on_style_text),
            element!("[style]", on_style_attribute),
            element!("img[src]", drop_bad_url("src")),
            element!("link[href]", drop_bad_url("href")),
        ],
        enable_esi_tags: false,
        ..RewriteStrSettings::new()
    };
    rewrite_str(html, settings).map_err(|e| match e {
        RewritingError::ContentHandlerError(e) if e.is::<Over>() => Error::PageDomLimit {
            message: format!("the page's CSS is too much for a printer: {e}"),
        },
        other => Error::Html(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screened(html: &str) -> Result<String, Error> {
        screen_page(html, false)
    }

    fn ok(html: &str) -> String {
        screened(html).unwrap_or_else(|e| panic!("{e}"))
    }

    fn over(html: &str) {
        match screened(html) {
            Err(Error::PageDomLimit { message }) => assert!(message.contains("CSS"), "{message}"),
            other => panic!("expected PageDomLimit, got {other:?}"),
        }
    }

    #[test]
    fn ordinary_css_goes_through_untouched() {
        for html in [
            "<p>x</p>",
            "<style>p { color: red; background: url(a.png) } @media print { b { all: unset } }</style>",
            "<style>@import \"a.css\"; @import url(b.css);</style>",
            "<p style=\"color: red; width: calc(1px + (2px * 3))\">x</p>",
            "<div style=\"grid-template-areas: 'a b' 'c d'\"></div>",
            "<img src=\"a.png\"><link rel=\"icon\" href=\"/x.png\">",
        ] {
            assert_eq!(ok(html), html);
        }
    }

    #[test]
    fn urls_a_printer_cant_resolve_go() {
        for (html, want) in [
            (
                "<style>@import \"http://[\";p{color:red}</style>",
                "<style>@import \"data:,\";p{color:red}</style>",
            ),
            (
                "<style>@import url(http://[);</style>",
                "<style>@import url(data:,);</style>",
            ),
            (
                "<style>@font-face{src:url(\"http://[\")}</style>",
                "<style>@font-face{src:url(\"data:,\")}</style>",
            ),
            (
                "<p style=\"background:url(http://[)\">x</p>",
                "<p style=\"background:url(data:,)\">x</p>",
            ),
            ("<img src=\"http://[\">", "<img>"),
            (
                "<link rel=\"stylesheet\" href=\"http://[\">",
                "<link rel=\"stylesheet\">",
            ),
            ("<img src=\"http://&#91;\">", "<img>"),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
    }

    #[test]
    fn a_page_cant_outspend_the_budget() {
        over(&format!(
            "<style>{}</style>",
            "a{}".repeat(MAX_CSS_BYTES / 3 + 1)
        ));
        let attr = format!("<p style=\"{}\"></p>", "color:red;".repeat(1000));
        over(&attr.repeat(MAX_CSS_BYTES / 10_000 + 1));
        over(&format!(
            "<style>a{{{}}}</style>",
            "--x:1;".repeat(MAX_BLOCK_DECLARATIONS + 1)
        ));
        // 60 KB of custom properties and `all`, the shape from crates/dom.
        over(&format!(
            "<style>a{{{}{}}}</style>",
            "--x:1;".repeat(3_000),
            "all:inherit;".repeat(1_000)
        ));
        // Blocks that fit, adding up.
        let rule = format!("a{{{}}}", "--x:1;".repeat(4_000));
        over(&format!("<style>{}</style>", rule.repeat(5)));
        over(&format!(
            "<p style=\"{}\"></p>",
            "(".repeat(MAX_NESTING + 2)
        ));
        over(&format!("<style>{}</style>", "a{".repeat(MAX_NESTING + 2)));
        let grid = format!("'{}'", "a ".repeat(MAX_CELLS + 1));
        over(&format!("<p style=\"grid-template-areas:{grid}\"></p>"));
        // Carried in a custom property for a var() to fetch.
        over(&format!("<style>a{{--g:{grid}}}</style>"));
        let cells = format!("<p style=\"--g:'{}'\"></p>", "a ".repeat(MAX_CELLS));
        over(&cells.repeat(MAX_PAGE_CELLS / MAX_CELLS + 1));
    }

    #[test]
    fn scripts_and_noscript_go_however_they_got_here() {
        const ISLAND: &str =
            "<script type=\"application/vnd.surimono+json\" data-renderer=\"t\">{}</script>";
        for (html, want) in [
            // As a comment's text ended early by what it held.
            ("<!----><script>alert(1)</script><!---->", "<!----><!---->"),
            ("<!----><noscript>NS</noscript><!---->", "<!----><!---->"),
            // As a raw-text element's.
            ("<style></style><script>x</script>", "<style></style>"),
            ("<xmp></xmp><noscript>x</noscript>", "<xmp></xmp>"),
            ("<iframe></iframe><script>x</script>", "<iframe></iframe>"),
            // An HTML style in SVG reparses as foreign content.
            (
                "<svg><style><script>x</script></style></svg>",
                "<svg><style></style></svg>",
            ),
            // Upper case is the same element to a parser.
            ("<SCRIPT>x</SCRIPT><NOSCRIPT>y</NOSCRIPT>z", "z"),
            (ISLAND, ISLAND),
            // A marker whose text closes the script: what follows is no marker.
            (
                "<script type=\"application/vnd.surimono+json\">{}</script><script>x</script>",
                "<script type=\"application/vnd.surimono+json\">{}</script>",
            ),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
        let inert =
            "<template><script>x</script><noscript>y</noscript></template><script>z</script>";
        assert_eq!(
            ok(inert),
            "<template><script>x</script><noscript>y</noscript></template>"
        );
        let kept = screen_page("<script>x</script><noscript>y</noscript>", true);
        assert_eq!(kept.ok().as_deref(), Some("<script>x</script>"));
    }

    #[test]
    fn strings_elsewhere_arent_cells() {
        let text = format!("<p style=\"content:'{}'\"></p>", "a ".repeat(MAX_CELLS + 1));
        assert_eq!(ok(&text), text);
    }

    #[test]
    fn a_style_element_over_several_text_chunks_is_read_whole() {
        // Longer than lol_html's buffers, with the bad URL at the end.
        let css = format!("{}@import \"http://[\";", "p{color:red}".repeat(100_000));
        let out = ok(&format!("<style>{css}</style>"));
        assert!(
            out.ends_with("@import \"data:,\";</style>"),
            "{}",
            &out[out.len() - 80..]
        );
        assert_eq!(out.len(), css.len() + "<style></style>".len() - 2);
    }
}
