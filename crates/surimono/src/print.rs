//! The last step of a page: what a printer may safely be given.
//!
//! The tree is clean by the time it is serialized, but the text isn't: the
//! serializer writes comments and raw-text elements unescaped, as the standard
//! says, so a comment holding `--><script>…` or a `<style>` holding
//! `</style><noscript>…` becomes markup in whoever parses the result. And a
//! printer doesn't read a page the way the page ran: it parses with scripting
//! off, so `<noscript>` holds markup, its templates aren't fragments, and
//! `<template><noscript></template>…` opens onto real elements.
//!
//! So the only reading that counts is the printer's own, and [`finish_page`]
//! works on that tree ([`tree`]) and on nothing else. It parses the serialized
//! page as the printer will, screens what the parser produced, and writes the
//! result from the tree. That output is then parsed and screened again, until
//! a pass changes nothing: what is handed over is a fixed point, so the tree
//! the printer builds from it is one that passed every screen. Parsing can
//! still rearrange what a tree writes out (a `<form>` in a `<form>`, which no
//! parser builds but a script can), which a pass or two settles; a page that
//! doesn't settle in [`MAX_PASSES`] fails with
//! [`Error::PageFailed`](crate::Error::PageFailed).
//!
//! - **Tree:** [`MAX_NODES`] and [`MAX_DEPTH`] hold on the printer's parse, as
//!   the document's defaults do on the page's, and
//!   [`MAX_ATTRIBUTES`] per element.
//! - **Scripts and `<noscript>`:** removed here too, as they are from the tree
//!   (`cleanup.js`), in any namespace. Inside an HTML `<template>` they stay,
//!   inert in any printer. An `<svg><template>` isn't one. Island markers stay,
//!   and [`Options::keep_scripts`](crate::Options::keep_scripts) keeps scripts.
//!
//! A printer's CSS engine isn't under any limit of ours. fulgur hands the page
//! to blitz-dom and stylo, whose parse can't be interrupted, in the caller's
//! process, after surimono has returned. Scripts can write whatever they like
//! into `<style>` and `style` attributes, so everything that reaches stylo goes
//! through here first, with one budget for the whole page: `crates/dom` bounds
//! a single attribute, and many of those add up. Everywhere in the tree,
//! templates too, because blitz-dom gives a `<style>` in a template to stylo.
//! Its CSS is what blitz-dom reads: all the text under the element in any
//! namespace, with character references decoded, even in HTML's raw text.
//!
//! - **Size:** [`MAX_CSS_BYTES`] over all style text, and an attribute's own
//!   [`MAX_ATTRIBUTE_BYTES`] and [`MAX_ATTRIBUTE_TOKENS`].
//! - **Declarations:** a push of a property a block already has walks the
//!   block, and `all` pushes every longhand, so a block costs its size squared,
//!   and `all` multiplies it; 60 KB took 11 s (`crates/dom/DESIGN.md`).
//!   [`MAX_BLOCK_DECLARATIONS`] per block, [`MAX_STEPS`] over the page.
//! - **Grid strings:** `grid-template-areas` tokenizes its strings again and
//!   interns each name, and `var()` can carry such a string out of a custom
//!   property, so strings count in both. [`MAX_CELLS`] per declaration,
//!   [`MAX_PAGE_CELLS`] over the page.
//! - **Nesting:** [`MAX_NESTING`] brackets, for the recursion in stylo.
//! - **References:** a custom property's `var()`s name others, and a chain of
//!   them is followed link by link. [`MAX_VAR_CHAIN`] over the page, counting
//!   every declaration of a name together, and every link in a cycle.
//! - **Selectors:** [`MAX_COMBINATORS`] in one.
//!
//! The last two are set far below any stack: real pages have a handful of each.
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
//! All of this runs under a [`Gate`], one deadline and the cancel flag, and the
//! output under [`Limits::output_bytes`](crate::Limits::output_bytes).
//!
//! What isn't bounded here: matching selectors against the tree, which
//! happens in the printer and depends on both. Run your printer in a process of
//! its own if pages come from people you don't trust: a timeout can't stop a
//! thread, or catch what ends a process.

mod tree;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cssparser::{Parser, ParserInput, Token};
use html_escape::decode_html_entities;
use surimono_dom::DomLimits;
use url::Url;

use crate::error::Error;
use crate::prepass::is_island_type;
use crate::resources::{Fetcher, inline_stylesheets};
pub(crate) use tree::Tree;
use tree::TreeLimits;
pub(crate) use tree::too_long;

/// Most times the output is parsed and screened again. Each level of markup
/// that only a reparse uncovers takes one; real pages take two passes.
const MAX_PASSES: usize = 4;
/// Most nodes the printer's parse may build, the document's default.
const MAX_NODES: usize = DomLimits::DEFAULT_MAX_NODES;
/// Deepest nesting of the printer's tree, the document's default.
const MAX_DEPTH: usize = DomLimits::DEFAULT_MAX_DEPTH;
/// Most attributes on one element. html5ever and blitz-dom compare each new
/// attribute of a tag with the others, so one tag's cost is this squared.
const MAX_ATTRIBUTES: usize = 4_096;

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
/// Most bytes in one `style` attribute, as `crates/dom` has.
const MAX_ATTRIBUTE_BYTES: usize = 8 << 20;
/// Most tokens in one `style` attribute, as `crates/dom` has.
const MAX_ATTRIBUTE_TOKENS: usize = 1 << 16;
/// Longest chain of custom properties naming each other through `var()`. Design
/// tokens have under ten links (primitive, semantic, component, state). Each link
/// may take a stack frame in a CSS engine: at an assumed 2 KiB each, a generous
/// figure, 64 links are 128 KiB, an eighth of a Ruby thread's 1 MiB.
const MAX_VAR_CHAIN: usize = 64;
/// Most combinators in one selector, by the same reckoning: handwritten CSS has
/// under ten and generated nesting under thirty.
const MAX_COMBINATORS: usize = 64;
/// Tokens between looks at the gate.
const GATE_TOKENS: usize = 1 << 12;

/// When post-run work has to stop: one deadline over all of it, and the
/// caller's cancel flag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gate<'a> {
    stop: &'a AtomicBool,
    deadline: Instant,
}

impl<'a> Gate<'a> {
    pub(crate) fn new(stop: &'a AtomicBool, time: Duration) -> Self {
        Self {
            stop,
            deadline: Instant::now() + time,
        }
    }

    /// [`Error::Cancelled`] or [`Error::PageTimeLimit`] once it is time to stop.
    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.stop.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(Error::PageTimeLimit);
        }
        Ok(())
    }
}

/// What's left of the page's budget.
#[derive(Debug, Default)]
struct Budget {
    bytes: usize,
    steps: u64,
    cells: usize,
    over: Option<&'static str>,
    /// The gate said stop.
    halted: Option<Error>,
    references: References,
}

/// Which custom properties name which, over the whole page.
#[derive(Debug, Default)]
struct References {
    ids: HashMap<String, usize>,
    /// By id: the ids its declarations name, with repeats.
    names: Vec<Vec<usize>>,
}

impl References {
    fn id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len();
        self.ids.insert(name.to_owned(), id);
        self.names.push(Vec::new());
        id
    }

    /// The longest chain there can be: the most links that end in a property
    /// naming nothing, peeling those off layer by layer, plus every property
    /// left, which are on or lead to a cycle.
    fn longest_chain(&self) -> usize {
        let mut waiting: Vec<usize> = self.names.iter().map(Vec::len).collect();
        let mut named_by = vec![Vec::new(); self.names.len()];
        for (id, names) in self.names.iter().enumerate() {
            for &name in names {
                named_by[name].push(id);
            }
        }
        let mut links = vec![0; self.names.len()];
        let mut ready: Vec<usize> = (0..waiting.len()).filter(|&i| waiting[i] == 0).collect();
        let mut peeled = 0;
        let mut longest = 0;
        while let Some(id) = ready.pop() {
            peeled += 1;
            longest = longest.max(links[id]);
            for &by in &named_by[id] {
                links[by] = links[by].max(links[id] + 1);
                waiting[by] -= 1;
                if waiting[by] == 0 {
                    ready.push(by);
                }
            }
        }
        longest + (self.names.len() - peeled)
    }
}

impl Budget {
    fn over(&mut self, what: &'static str) {
        self.over.get_or_insert(what);
    }

    fn running(&self) -> bool {
        self.over.is_none() && self.halted.is_none()
    }

    /// What stopped the screen, if anything did.
    fn stopped(&mut self) -> Result<(), Error> {
        if let Some(e) = self.halted.take() {
            return Err(e);
        }
        match self.over {
            Some(what) => Err(Error::PageDomLimit {
                message: format!("the page's CSS is too much for a printer: {what}"),
            }),
            None => Ok(()),
        }
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
    /// How deep this block's own tokens are read.
    depth: usize,
    /// The custom property being declared, if one is.
    custom: Option<usize>,
    /// The custom property whose value this block is part of, if any: all that
    /// is in here is that property's.
    within: Option<usize>,
    /// Combinators so far in the selector this may turn out to be, and whether
    /// the next token comes after one.
    combinators: usize,
    after_gap: bool,
    /// A list in a pseudo-class goes on: space after its commas is no combinator.
    after_comma: bool,
}

impl Block {
    fn new(depth: usize, within: Option<usize>) -> Self {
        Self {
            at_name: true,
            depth,
            custom: within,
            within,
            ..Self::default()
        }
    }

    fn end_declaration(&mut self) {
        self.at_name = true;
        self.counts_cells = false;
        self.cells = 0;
        self.custom = self.within;
        self.combinators = 0;
        self.after_gap = false;
    }

    /// Notes a token of a selector, if this turns out to be one.
    fn note_token(&mut self, depth: usize, token: &Token<'_>) {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => self.after_gap |= !self.after_comma,
            Token::Delim('>' | '+' | '~') => self.after_gap = true,
            Token::Comma => {
                if depth == self.depth {
                    self.combinators = 0;
                }
                self.after_gap = false;
                self.after_comma = true;
            }
            _ => {
                self.combinators += usize::from(self.after_gap);
                self.after_gap = false;
                self.after_comma = false;
            }
        }
    }
}

/// The URL as it would be resolved, if it can be.
fn resolves(url: &str) -> bool {
    Url::parse("file:///").is_ok_and(|base| base.join(url).is_ok())
}

/// What the first token inside a block means.
#[derive(Debug, Clone, Copy)]
enum First {
    Url,
    Reference,
    Anything,
}

/// Reads tokens up to the end of this parser's block.
struct Screen<'b> {
    budget: &'b mut Budget,
    gate: &'b Gate<'b>,
    /// Tokens read, and how many are allowed.
    tokens: usize,
    max_tokens: usize,
    /// Byte ranges of `input` to replace, with their replacements.
    fixes: Vec<(Range<usize>, &'static str)>,
}

impl Screen<'_> {
    fn walk(&mut self, p: &mut Parser<'_, '_>, block: &mut Block, depth: usize) {
        // Whether the next string is an `@import`'s URL, or a `url(`'s.
        let mut url_next = false;
        while self.budget.running() {
            let start = p.position().byte_index();
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                return;
            };
            self.tokens += 1;
            if self.tokens > self.max_tokens {
                self.budget.over("too many tokens in a style attribute");
            }
            if self.tokens.is_multiple_of(GATE_TOKENS) {
                self.budget.halted = self.gate.check().err();
            }
            let token = token.clone();
            let end = p.position().byte_index();
            block.note_token(depth, &token);
            match &token {
                Token::WhiteSpace(_) | Token::Comment(_) => continue,
                Token::Semicolon => {
                    block.declarations += 1;
                    if depth == block.depth {
                        block.end_declaration();
                    }
                    url_next = false;
                    continue;
                }
                Token::Ident(name) if block.at_name => {
                    block.at_name = false;
                    block.alls += u64::from(name.eq_ignore_ascii_case("all"));
                    if name.starts_with("--") && block.within.is_none() {
                        block.custom = Some(self.budget.references.id(name));
                    }
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
            let first = match &token {
                Token::Function(f) if f.eq_ignore_ascii_case("url") => First::Url,
                Token::Function(f) if f.eq_ignore_ascii_case("var") => First::Reference,
                _ => First::Anything,
            };
            url_next = false;
            match token {
                Token::CurlyBracketBlock => {
                    if block.combinators > MAX_COMBINATORS {
                        self.budget.over("too many combinators in a selector");
                    }
                    let mut inner = Block::new(depth + 1, block.custom);
                    self.nested(p, &mut inner, depth, First::Anything);
                    self.close(&inner);
                    block.end_declaration();
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                    self.nested(p, block, depth, first)
                }
                _ => {}
            }
        }
    }

    fn nested(&mut self, p: &mut Parser<'_, '_>, block: &mut Block, depth: usize, first: First) {
        if depth >= MAX_NESTING {
            self.budget.over("nested too deeply");
            return;
        }
        let result: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
            match first {
                First::Url => self.first_string_is_url(p),
                First::Reference => self.first_ident_is_named(p, block),
                First::Anything => {}
            }
            self.walk(p, block, depth + 1);
            Ok(())
        });
        drop(result);
    }

    /// `var(--name…)`: what the property being declared depends on.
    fn first_ident_is_named(&mut self, p: &mut Parser<'_, '_>, block: &Block) {
        let state = p.state();
        if let (Some(from), Ok(Token::Ident(name))) = (block.custom, p.next()) {
            let to = self.budget.references.id(name);
            self.budget.references.names[from].push(to);
        }
        p.reset(&state);
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

/// `css` with what mustn't reach a printer replaced, if anything had to be,
/// charged to `budget`. `attribute` for the text of a `style` attribute, which
/// is one block.
fn screen_css(
    css: &str,
    attribute: bool,
    budget: &mut Budget,
    gate: &Gate<'_>,
) -> Result<Option<String>, Error> {
    budget.bytes = budget.bytes.saturating_add(css.len());
    if budget.bytes > MAX_CSS_BYTES || (attribute && css.len() > MAX_ATTRIBUTE_BYTES) {
        budget.over("too much CSS");
        return budget.stopped().map(|()| None);
    }
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut screen = Screen {
        budget,
        gate,
        tokens: 0,
        max_tokens: if attribute {
            MAX_ATTRIBUTE_TOKENS
        } else {
            usize::MAX
        },
        fixes: Vec::new(),
    };
    let mut top = Block::new(0, None);
    top.at_name = attribute;
    screen.walk(&mut parser, &mut top, 0);
    if attribute {
        screen.close(&top);
    }
    let fixes = screen.fixes;
    budget.stopped()?;
    if fixes.is_empty() {
        return Ok(None);
    }
    let mut out = String::with_capacity(css.len());
    let mut at = 0;
    for (range, with) in fixes {
        out.push_str(&css[at..range.start]);
        out.push_str(with);
        at = range.end;
    }
    out.push_str(&css[at..]);
    Ok(Some(out))
}

/// What one pass does to a node.
struct Visit {
    id: usize,
    depth: usize,
    /// Inside an HTML `<template>`.
    inert: bool,
}

/// Whether `element` goes, outside a template.
fn must_go(element: &tree::Element, keep_scripts: bool) -> bool {
    match element.local() {
        "noscript" => true,
        "script" => {
            let island =
                element.is_html("script") && element.attr("type").is_some_and(is_island_type);
            !keep_scripts && !island
        }
        _ => false,
    }
}

/// The `style` attributes on `id`, and the URLs a printer would panic on.
fn screen_attributes(
    tree: &mut Tree,
    id: usize,
    budget: &mut Budget,
    gate: &Gate<'_>,
) -> Result<(), Error> {
    let Some(element) = tree.element_mut(id) else {
        return Ok(());
    };
    // A media list is CSS to a printer's engine, and holds no rule.
    let has_media = matches!(element.local(), "link" | "style");
    if has_media {
        element
            .attrs
            .retain(|a| a.name.local.as_ref() != "media" || !a.value.contains([';', '{', '}']));
    }
    for a in &mut element.attrs {
        let attribute = match a.name.local.as_ref() {
            "style" => true,
            "media" if has_media => false,
            _ => continue,
        };
        if let Some(fixed) = screen_css(&a.value, attribute, budget, gate)? {
            a.value = fixed.as_str().into();
        }
    }
    let url_attribute = match element.local() {
        "img" => "src",
        "link" => "href",
        _ => return Ok(()),
    };
    element
        .attrs
        .retain(|a| a.name.local.as_ref() != url_attribute || resolves(&a.value));
    Ok(())
}

/// The CSS of the `<style>` at `id`, as blitz-dom will read it.
fn screen_style(
    tree: &mut Tree,
    id: usize,
    budget: &mut Budget,
    gate: &Gate<'_>,
) -> Result<(), Error> {
    let text = tree.text_of(id);
    if text.len() > MAX_CSS_BYTES {
        budget.over("too much CSS");
        return budget.stopped();
    }
    let css = decode_html_entities(&text);
    let fixed = screen_css(&css, false, budget, gate)?;
    let foreign = tree
        .element(id)
        .is_some_and(|e| e.name.ns != html5ever::ns!(html));
    if fixed.is_some() || (foreign && css.contains("</")) {
        // Decoded, it may hold what would end the element; `\/` is `/` in CSS.
        let css = fixed.map_or(css, Cow::Owned);
        tree.set_text(id, css.replace("</", "<\\/"));
    }
    Ok(())
}

/// One pass over the tree; see the module docs.
fn screen_tree(tree: &mut Tree, keep_scripts: bool, gate: &Gate<'_>) -> Result<(), Error> {
    let mut budget = Budget::default();
    let mut stack = vec![Visit {
        id: 0,
        depth: 0,
        inert: false,
    }];
    while let Some(Visit { id, depth, inert }) = stack.pop() {
        gate.check()?;
        if depth > MAX_DEPTH {
            return Err(Error::PageDomLimit {
                message: format!("the page would nest over {MAX_DEPTH} levels"),
            });
        }
        let mut inside = inert;
        if let Some(element) = tree.element(id) {
            if !inert && must_go(element, keep_scripts) {
                tree.detach(id);
                continue;
            }
            inside |= element.is_html("template");
            let is_style = element.local() == "style";
            screen_attributes(tree, id, &mut budget, gate)?;
            if is_style {
                screen_style(tree, id, &mut budget, gate)?;
            }
        }
        stack.extend(tree.nodes[id].children.iter().rev().map(|&id| Visit {
            id,
            depth: depth + 1,
            inert: inside,
        }));
    }
    if budget.references.longest_chain() > MAX_VAR_CHAIN {
        budget.over("too long a chain of var() references");
    }
    budget.stopped()
}

/// How [`finish_page`] goes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Finish {
    /// [`Options::keep_scripts`](crate::Options::keep_scripts).
    pub(crate) keep_scripts: bool,
    /// [`Limits::output_bytes`](crate::Limits::output_bytes).
    pub(crate) max_bytes: usize,
}

/// A serialized page as a printer may have it; see the module docs. With a
/// loader, `<link>`s become `<style>`s first, on the printer's tree, so what
/// they load meets every screen.
pub(crate) fn finish_page(
    html: &str,
    finish: Finish,
    fetcher: Option<&mut Fetcher<'_>>,
    gate: &Gate<'_>,
) -> Result<String, Error> {
    if html.len() > finish.max_bytes {
        return Err(too_long(finish.max_bytes));
    }
    let limits = TreeLimits {
        max_nodes: MAX_NODES,
        max_depth: MAX_DEPTH,
        max_attributes: MAX_ATTRIBUTES,
    };
    let pass = |tree: &mut Tree| -> Result<String, Error> {
        screen_tree(tree, finish.keep_scripts, gate)?;
        tree.to_html(finish.max_bytes, gate)
    };
    let mut tree = Tree::parse(html, limits, gate)?;
    if let Some(fetcher) = fetcher {
        inline_stylesheets(&mut tree, fetcher, gate)?;
    }
    let mut out = pass(&mut tree)?;
    drop(tree);
    for _ in 0..MAX_PASSES {
        let again = pass(&mut Tree::parse(&out, limits, gate)?)?;
        if again == out {
            return Ok(out);
        }
        out = again;
    }
    Err(Error::PageFailed {
        message: format!("the page's markup still changed after {MAX_PASSES} parses"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: usize = usize::MAX / 8;

    fn finish(html: &str, keep_scripts: bool, max_bytes: usize) -> Result<String, Error> {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let finish = Finish {
            keep_scripts,
            max_bytes,
        };
        finish_page(html, finish, None, &gate)
    }

    fn document(body: &str) -> String {
        format!("<!doctype html><html><head></head><body>{body}</body></html>")
    }

    fn screened(body: &str) -> Result<String, Error> {
        finish(&document(body), false, WIDE)
    }

    /// What's in `<body>` after the page's checks.
    fn ok(body: &str) -> String {
        let out = screened(body).unwrap_or_else(|e| panic!("{e}"));
        let inner = out
            .strip_prefix("<!doctype html><html><head></head><body>")
            .and_then(|o| o.strip_suffix("</body></html>"));
        inner.unwrap_or(&out).to_owned()
    }

    fn over(body: &str) {
        match screened(body) {
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
            "<style>a::before { content: \"&lt;\" }</style>",
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
            // The parser's `image` is `img`.
            ("<image src=\"http://[\">", "<img>"),
            (
                "<svg><link href=\"http://[\"></svg>",
                "<svg><link></link></svg>",
            ),
            // blitz-dom decodes style text, even in HTML's raw text.
            (
                "<style>@import &quot;http://[&quot;;</style>",
                "<style>@import \"data:,\";</style>",
            ),
            // A printer's templates are real children.
            (
                "<template><style>@import 'http://[';</style></template>",
                "<template><style>@import \"data:,\";</style></template>",
            ),
            (
                "<template><img src=\"http://[\"></template>",
                "<template><img></template>",
            ),
            // An SVG style is markup: the parser decodes it, and it is escaped
            // going back.
            (
                "<svg><style>@import &quot;http://[&quot;;</style></svg>",
                "<svg><style>@import \"data:,\";</style></svg>",
            ),
            // All the text under it, as text_content.
            (
                "<svg><style>@import <a>'http://['</a>;</style></svg>",
                "<svg><style>@import \"data:,\";</style></svg>",
            ),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
    }

    #[test]
    fn decoded_style_text_cant_end_its_element() {
        let out =
            ok("<style>@import &quot;http://[&quot;;a{b:&lt;/style&gt;&lt;script&gt;}</style>");
        assert_eq!(
            out,
            "<style>@import \"data:,\";a{b:<\\/style><script>}</style>"
        );
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
        // One attribute's own limits, as crates/dom has them.
        over(&format!(
            "<p style=\"color:{}\"></p>",
            "a,".repeat(MAX_ATTRIBUTE_TOKENS)
        ));
        over(&format!(
            "<p style=\"background:url({})\"></p>",
            "a".repeat(MAX_ATTRIBUTE_BYTES + 1)
        ));
    }

    #[test]
    fn the_budget_reads_the_printers_css() {
        // References the parser decodes in SVG, and blitz-dom in HTML, where
        // it wants the `;`.
        let semi = "--x:1&#59 ".repeat(3_000) + &"all:inherit&#59 ".repeat(1_000);
        over(&format!("<svg><style>a{{{semi}}}</style></svg>"));
        let semi = "--x:1&#59;".repeat(3_000) + &"all:inherit&#59;".repeat(1_000);
        over(&format!("<style>a{{{semi}}}</style>"));
        // In a `<noscript>`, which is markup to a printer.
        let css = format!("{}{}", "--x:1;".repeat(3_000), "all:inherit;".repeat(1_000));
        over(&format!(
            "<template><noscript></template><style>a{{{css}}}</style></noscript>"
        ));
    }

    #[test]
    fn scripts_and_noscript_go_however_they_got_here() {
        const ISLAND: &str =
            "<script type=\"application/vnd.surimono+json\" data-renderer=\"t\">{}</script>";
        for (html, want) in [
            // As a comment's text ended early by what it held.
            ("<!----><script>alert(1)</script><!---->", ""),
            ("<!----><noscript>NS</noscript><!---->", ""),
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
            // A noscript as a printer reads it, opened in a template.
            (
                "<template><noscript></template><script>alert(1)</script>\
                 <noscript>NS</noscript></noscript>x",
                "<template><noscript></noscript></template>x",
            ),
            // No template in SVG: a real script in a real tree.
            (
                "<svg><template><script>alert(1)</script><noscript>NS</noscript></template></svg>",
                "<svg><template></template></svg>",
            ),
            // No integration point in blitz-html: a style holding markup.
            (
                "<math><annotation-xml encoding=text/html><style><script>alert(1)</script></style>\
                 </annotation-xml></math>",
                "<math><annotation-xml encoding=\"text/html\"><style></style></annotation-xml></math>",
            ),
            // A nested foreign script, with its own namespace.
            ("<svg><script>alert(1)</script></svg>", "<svg></svg>"),
            ("<svg><noscript>a</noscript></svg>", "<svg></svg>"),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
        let inert =
            "<template><script>x</script><noscript>y</noscript></template><script>z</script>";
        assert_eq!(
            ok(inert),
            "<template><script>x</script><noscript>y</noscript></template>"
        );
        let kept = finish(
            &document("<script>x</script><noscript>y</noscript>"),
            true,
            WIDE,
        );
        assert_eq!(
            kept.ok().as_deref(),
            Some("<!doctype html><html><head></head><body><script>x</script></body></html>")
        );
    }

    #[test]
    fn limits_hold_on_the_printers_parse() {
        let limit = |body: &str| match screened(body) {
            Err(Error::PageDomLimit { .. }) => {}
            other => panic!("expected PageDomLimit, got {other:?}"),
        };
        // Past the document's depth in what the page's own parse called text.
        limit(&format!(
            "<template><noscript></template>{}</noscript>",
            "<div>".repeat(20_000)
        ));
        limit(&"<div>".repeat(MAX_DEPTH + 1));
        limit(&"<br>".repeat(MAX_NODES + 1));
        // Moved deep by the parser after it was placed.
        limit(&format!("{}{}", "<b>".repeat(600), "<p>x</b>"));
        limit(&format!(
            "<p {}></p>",
            (0..=MAX_ATTRIBUTES)
                .map(|i| format!("a{i}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        assert_eq!(
            ok(&"<div>".repeat(MAX_DEPTH - 4)).matches("</div>").count(),
            MAX_DEPTH - 4
        );
    }

    #[test]
    fn the_output_is_a_fixed_point() {
        for (html, want) in [
            // The parser drops a first newline.
            ("<pre>\n\nx</pre>", "<pre>\n\nx</pre>"),
            ("<textarea>\n\nx</textarea>", "<textarea>\n\nx</textarea>"),
            // A literal one would come back a newline.
            ("<p>a&#13;b</p>", "<p>a&#13;b</p>"),
            ("<p title=\"a&#13;b\"></p>", "<p title=\"a&#13;b\"></p>"),
            // Nothing can close it.
            ("<p>a</p><plaintext>x</b>", "<p>a</p><plaintext>x</b>"),
            // Past a comment, which goes.
            ("<pre><!--c-->\nx</pre>", "<pre>\n\nx</pre>"),
            // Names the parser adjusts in foreign content.
            (
                "<svg xmlns:xlink=\"a\" xlink:href=\"b\" xml:lang=\"c\" viewBox=\"0 0 1 1\"></svg>",
                "<svg xmlns:xlink=\"a\" xlink:href=\"b\" xml:lang=\"c\" viewBox=\"0 0 1 1\"></svg>",
            ),
            // What no parser builds but a tree can hold.
            (
                "<form><div></form><form>x</form>",
                "<form><div>x</div></form>",
            ),
        ] {
            let out = finish(&document(html), false, WIDE).unwrap_or_else(|e| panic!("{e}"));
            assert!(out.contains(want), "{html}: {out}");
            let again = finish(&out, false, WIDE).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(again, out, "{html}");
        }
    }

    #[test]
    fn no_byte_order_mark_stays_in_the_output() {
        let mark = '\u{feff}';
        for html in [
            format!("<p>a{mark}b</p>"),
            format!("<p title=\"a{mark}b\" a{mark}b=\"c\"></p>"),
            format!("<a{mark}b>x</a{mark}b>"),
            format!("<style>p{mark}{{color:red}}</style>"),
            format!("<xmp>a{mark}b</xmp>"),
            format!("<svg><style>p{mark}{{color:red}}</style></svg>"),
        ] {
            let out = finish(&document(&html), false, WIDE).unwrap_or_else(|e| panic!("{e}"));
            assert!(!out.contains(mark), "{html}: {out}");
            assert_eq!(
                finish(&out, false, WIDE).as_ref().ok(),
                Some(&out),
                "{html}"
            );
        }
        assert_eq!(ok(&format!("<p>a{mark}b</p>")), "<p>a&#xfeff;b</p>");
    }

    #[test]
    fn a_media_list_holds_no_rule() {
        for (html, want) in [
            (
                "<link rel=\"a\" media=\"print\">",
                "<link rel=\"a\" media=\"print\">",
            ),
            (
                "<style media=\"print\"></style>",
                "<style media=\"print\"></style>",
            ),
            ("<link rel=\"a\" media=\"a{b:c}\">", "<link rel=\"a\">"),
            ("<link rel=\"a\" media=\"a;b\">", "<link rel=\"a\">"),
            ("<style media=\"}\"></style>", "<style></style>"),
            // Other elements' `media` is no CSS to a printer.
            ("<source media=\"a;b\">", "<source media=\"a;b\">"),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
        over(&format!(
            "<link rel=\"a\" media=\"{}\">",
            "(".repeat(MAX_NESTING + 2)
        ));
    }

    #[test]
    fn a_foreign_style_has_no_end_tag_in_it() {
        for (html, want) in [
            (
                "<svg><style>a{b:&lt;/style&gt;}</style></svg>",
                "<svg><style>a{b:&lt;\\/style&gt;}</style></svg>",
            ),
            (
                "<math><style>a{b:&lt;/c}</style></math>",
                "<math><style>a{b:&lt;\\/c}</style></math>",
            ),
        ] {
            assert_eq!(ok(html), want, "{html}");
        }
        // HTML's raw text stays as it was.
        assert_eq!(ok("<style>a{b:</c}</style>"), "<style>a{b:</c}</style>");
    }

    fn chain(links: usize) -> String {
        let declarations: String = (0..links)
            .map(|i| format!("--v{}:var(--v{i});", i + 1))
            .collect();
        format!("<style>:root{{--v0:1;{declarations}}}</style>")
    }

    #[test]
    fn references_chain_only_so_far() {
        assert_eq!(ok(&chain(MAX_VAR_CHAIN)), chain(MAX_VAR_CHAIN));
        over(&chain(MAX_VAR_CHAIN + 1));
        // Across elements: a name is one name.
        let split: String = (0..=MAX_VAR_CHAIN)
            .map(|i| format!("<p style=\"--v{}:var(--v{i})\"></p>", i + 1))
            .collect();
        over(&split);
        // Inside a fallback, a bracket or a block of the value.
        for value in ["var(--x,var(--v{i}))", "(;var(--v{i}))", "{a:var(--v{i})}"] {
            let links: String = (0..=MAX_VAR_CHAIN)
                .map(|i| format!("--v{}:{};", i + 1, value.replace("{i}", &i.to_string())))
                .collect();
            over(&format!("<style>a{{{links}}}</style>"));
        }
        // A cycle counts each of its links.
        let ring: String = (0..=MAX_VAR_CHAIN)
            .map(|i| format!("--v{i}:var(--v{});", (i + 1) % (MAX_VAR_CHAIN + 1)))
            .collect();
        over(&format!("<style>a{{{ring}}}</style>"));
        // Many references aren't a chain, and leaves aren't links.
        let names: String = (0..4000).map(|i| format!("--a{i}:var(--b);")).collect();
        let many = format!(
            "<style>a{{{names}{}}}</style>",
            "color:var(--c);".repeat(50)
        );
        assert_eq!(ok(&many), many);
        let own = "<style>a{--a:var(--a,red)}</style>";
        assert_eq!(ok(own), own);
    }

    #[test]
    fn selectors_have_only_so_many_combinators() {
        let rule = |n: usize, with: &str| format!("a{}{{}}", with.repeat(n));
        for with in [" b", ">b", " > b", "+b", "~b"] {
            let fits = format!("<style>{}</style>", rule(MAX_COMBINATORS, with));
            assert_eq!(ok(&fits), fits);
            over(&format!(
                "<style>{}</style>",
                rule(MAX_COMBINATORS + 1, with)
            ));
        }
        // In a nested rule, and past a list inside a pseudo-class.
        let long = rule(MAX_COMBINATORS + 1, " b");
        over(&format!("<style>@media print{{{long}}}</style>"));
        over(&format!("<style>a{{{long}}}</style>"));
        over(&format!("<style>a:is(b,c){long}</style>"));
        // A list in a pseudo-class is no chain.
        let items = format!(
            "<style>:is({}a){{}}</style>",
            "a, ".repeat(MAX_COMBINATORS * 2)
        );
        assert_eq!(ok(&items), items);
        // A comma starts another selector, and a declaration's spaces are none.
        let list = format!("<style>{}a{{}}</style>", "a b,".repeat(MAX_COMBINATORS * 2));
        assert_eq!(ok(&list), list);
        let spaces = "1px ".repeat(MAX_COMBINATORS * 2);
        let declaration = format!("<style>a{{margin:{spaces}}}</style>");
        assert_eq!(ok(&declaration), declaration);
    }

    #[test]
    fn no_doctype_stays_no_doctype() {
        let out = finish("<p>x", false, WIDE).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, "<html><head></head><body><p>x</p></body></html>");
    }

    #[test]
    fn comments_go() {
        assert_eq!(
            ok("a<!-- x -->b<svg><style>c<!--d-->e</style></svg>"),
            "ab<svg><style>ce</style></svg>"
        );
    }

    #[test]
    fn the_output_limit_is_a_typed_error() {
        let html = document(&"<p>x</p>".repeat(100));
        for max in [10, 50, 500] {
            match finish(&html, false, max) {
                Err(Error::PageDomLimit { message }) => {
                    assert!(message.contains("output limit"), "{message}");
                }
                other => panic!("{other:?}"),
            }
        }
        // A small input that a parse would write out as much longer.
        let cap = html.len() + 8;
        assert!(finish(&html, false, cap).is_ok());
    }

    #[test]
    fn the_gate_stops_every_step() {
        let html = document(&"<p style=\"color:red\">x</p>".repeat(1000));
        let finishing = Finish {
            keep_scripts: false,
            max_bytes: WIDE,
        };
        let stop = AtomicBool::new(true);
        let gate = Gate::new(&stop, Duration::from_secs(60));
        assert!(matches!(
            finish_page(&html, finishing, None, &gate),
            Err(Error::Cancelled)
        ));
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::ZERO);
        assert!(matches!(
            finish_page(&html, finishing, None, &gate),
            Err(Error::PageTimeLimit)
        ));
        // A tag that never ends is stopped between chunks.
        let tag = format!("<p {}", "a".repeat(1 << 20));
        assert!(matches!(
            finish_page(&tag, finishing, None, &gate),
            Err(Error::PageTimeLimit)
        ));
        // CSS: past its first look at the gate, a stop still gets through.
        let gate = Gate::new(&stop, Duration::from_millis(0));
        let mut budget = Budget::default();
        let long = "a ".repeat(GATE_TOKENS);
        assert!(matches!(
            screen_css(&long, false, &mut budget, &gate),
            Err(Error::PageTimeLimit)
        ));
    }

    #[test]
    fn strings_elsewhere_arent_cells() {
        let text = format!("<p style=\"content:'{}'\"></p>", "a ".repeat(MAX_CELLS + 1));
        assert_eq!(ok(&text), text);
    }

    #[test]
    fn a_style_element_over_several_chunks_is_read_whole() {
        // Longer than the parser's chunks, with the bad URL at the end.
        let css = format!("{}@import \"http://[\";", "p{color:red}".repeat(100_000));
        let out = ok(&format!("<style>{css}</style>"));
        assert!(
            out.ends_with("@import \"data:,\";</style>"),
            "{}",
            &out[out.len() - 80..]
        );
        assert_eq!(out.len(), css.len() + "<style></style>".len() - 2);
    }

    /// Markup that has to come out the same from any number of checks, and
    /// clean to the printer, whatever order it's in.
    #[test]
    fn scrambled_markup_settles_and_stays_clean() {
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        for _ in 0..1_500 {
            let html = tree::tests::scrambled(&mut seed);
            let Ok(out) = finish(&html, false, WIDE) else {
                continue;
            };
            assert_eq!(
                finish(&out, false, WIDE).as_ref().ok(),
                Some(&out),
                "{html}"
            );
            let limits = TreeLimits {
                max_nodes: MAX_NODES,
                max_depth: MAX_DEPTH,
                max_attributes: MAX_ATTRIBUTES,
            };
            let stop = AtomicBool::new(false);
            let gate = Gate::new(&stop, Duration::from_secs(60));
            let tree = Tree::parse(&out, limits, &gate).unwrap_or_else(|e| panic!("{e}"));
            let mut stack = vec![(0, false)];
            while let Some((id, inert)) = stack.pop() {
                let mut inside = inert;
                if let Some(e) = tree.element(id) {
                    assert!(
                        inert || !matches!(e.local(), "script" | "noscript"),
                        "{html}: {out}"
                    );
                    inside |= e.is_html("template");
                }
                stack.extend(tree.nodes[id].children.iter().map(|&c| (c, inside)));
            }
        }
    }
}
