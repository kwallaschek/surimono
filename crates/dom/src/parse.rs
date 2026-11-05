//! The HTML parser's tree sink: blitz-html's, with what it leaves out.
//!
//! [`Sink`] forwards tree building to blitz-html's `DocumentHtmlParser` and
//! adds what blitz drops or does badly:
//!
//! - comment data, which `NodeData::Comment` has no room for;
//! - the quirks mode, which blitz-html records but never hands on;
//! - quiet parse errors: blitz-html's `TreeSink::finish` prints each one
//!   with `println!`, so `finish` here doesn't call it;
//! - limits: the input is fed in chunks and feeding stops once the tree is
//!   over [`DomLimits`](crate::DomLimits). html5ever walks its stack of open
//!   elements for most start tags, so a very deep tree would also cost
//!   quadratic time inside one call that can't be interrupted;
//! - style text: see [`stash_style_text`];
//! - `style` attributes: blitz-dom hands each to stylo as its element is
//!   made, in a call nothing can stop. One over [`style_attr::fits`]'s limits
//!   ends the parse, and the rest stay away from stylo under another name
//!   until the tree is built ([`style_attr::hide`]).

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;

use blitz_dom::DocumentMutator;
use blitz_html::DocumentHtmlParser;
use html5ever::driver::{self, ParseOpts};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::TokenizerOpts;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilderOpts, TreeSink};
use html5ever::{Attribute, QualName};

use crate::document::Special;
use crate::style_attr;

/// Bytes of HTML fed between limit checks.
const CHUNK: usize = 4096;

/// What the sink learned, beyond the tree.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// (node, data): comment data, and the text of `<style>` elements.
    pub strings: Vec<(usize, String)>,
    /// (node, what it is): the doctype.
    pub specials: Vec<(usize, Special)>,
    /// (`<template>`, its content fragment).
    pub templates: Vec<(usize, usize)>,
    /// Nodes the tree builder took out of the tree. It puts most back; what
    /// has no parent at the end (`<frameset>` drops the `<body>`) is an
    /// orphan for the caller to free.
    pub removed: Vec<usize>,
    pub quirks: bool,
    /// Feeding stopped because the tree went over a limit.
    pub over_limit: bool,
    /// Feeding stopped at a `style` attribute over its limits.
    pub over_style: bool,
    /// Elements whose `style` attributes are hidden from blitz-dom.
    hidden_styles: Vec<usize>,
}

/// Stops feeding the parser once `max_nodes` were created or an element
/// sits at `max_depth`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParseLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
}

struct Sink<'m, 'doc> {
    inner: DocumentHtmlParser<'m, 'doc>,
    /// The node the tree builder sees as the document. For a fragment, a
    /// holder outside the tree: blitz-html hangs the parser's root element
    /// on the real document, where `<style>` and `<img>` would take effect
    /// before the fragment is ever inserted.
    document: usize,
    parsed: RefCell<Parsed>,
    /// Depth by node id for nodes the parser placed; 0 for others.
    depths: RefCell<Vec<u32>>,
    /// Greatest depth seen, and elements and comments created; shared with
    /// the loop feeding the parser, which owns the sink. Text doesn't count:
    /// one text node takes many appends, and every text node sits next to
    /// an element or comment, or is the only one.
    deepest: Rc<Cell<u32>>,
    created: Rc<Cell<usize>>,
    /// Every `<style>` created; shared with the caller like the counters.
    styles: Rc<RefCell<Vec<usize>>>,
    /// A `style` attribute was over its limits; shared the same way.
    over_style: Rc<Cell<bool>>,
}

impl Sink<'_, '_> {
    fn note_created(&self) {
        self.created.set(self.created.get() + 1);
    }

    /// Takes out the `style` attributes if any is over the limits, and
    /// notes it: feeding stops, and the parse fails. Otherwise hides them,
    /// and says whether there were any.
    fn screen_style(&self, attrs: &mut Vec<Attribute>) -> bool {
        let is_style = |a: &Attribute| a.name.local.as_ref() == "style";
        if attrs
            .iter()
            .any(|a| is_style(a) && !style_attr::fits(&a.value))
        {
            attrs.retain(|a| !is_style(a));
            self.over_style.set(true);
        }
        let mut any = false;
        for a in attrs.iter_mut().filter(|a| is_style(a)) {
            style_attr::hide(&mut a.name);
            any = true;
        }
        any
    }

    fn note_child(&self, parent: usize, child: usize) {
        let mut d = self.depths.borrow_mut();
        let need = parent.max(child) + 1;
        if d.len() < need {
            d.resize(need, 0);
        }
        let depth = d[parent].saturating_add(1);
        d[child] = depth;
        self.deepest.set(self.deepest.get().max(depth));
    }
}

impl<'m, 'doc> TreeSink for Sink<'m, 'doc> {
    type Output = Parsed;
    type Handle = usize;
    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    /// Doesn't call the inner `finish`, which prints every parse error.
    fn finish(self) -> Parsed {
        self.parsed.into_inner()
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        self.document
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        self.inner.elem_name(target)
    }

    fn create_element(
        &self,
        name: QualName,
        mut attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> usize {
        self.note_created();
        let hidden = self.screen_style(&mut attrs);
        let is_style = name.local.as_ref() == "style";
        let id = self.inner.create_element(name, attrs, flags);
        if is_style {
            self.styles.borrow_mut().push(id);
        }
        if hidden {
            self.parsed.borrow_mut().hidden_styles.push(id);
        }
        id
    }

    fn create_comment(&self, text: StrTendril) -> usize {
        self.note_created();
        let id = self.inner.create_comment(text.clone());
        self.parsed
            .borrow_mut()
            .strings
            .push((id, text.to_string()));
        id
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        self.note_created();
        self.inner.create_pi(target, data)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        if let NodeOrText::AppendNode(id) = &child {
            self.note_child(*parent, *id);
        }
        self.inner.append(parent, child);
    }

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        self.inner.append_before_sibling(sibling, new_node);
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        self.inner
            .append_based_on_parent_node(element, prev_element, child);
    }

    /// blitz-html ignores it. A comment node to blitz-dom, so it stays out
    /// of layout; `Document` knows it for what it is.
    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        self.note_created();
        let id = self.inner.create_comment(StrTendril::new());
        self.inner
            .append(&self.document, NodeOrText::AppendNode(id));
        self.parsed.borrow_mut().specials.push((
            id,
            Special::Doctype {
                name: name.to_string(),
                public_id: public.to_string(),
                system_id: system.to_string(),
            },
        ));
    }

    /// HTML §4.12.3: a template's content is a fragment outside the tree.
    /// blitz-html answers with the template itself.
    fn get_template_contents(&self, target: &usize) -> usize {
        let mut parsed = self.parsed.borrow_mut();
        if let Some(&(_, content)) = parsed.templates.iter().find(|(t, _)| t == target) {
            return content;
        }
        let name = QualName::new(None, html5ever::ns!(), crate::document::fragment_name());
        let content = self
            .inner
            .create_element(name, Vec::new(), ElementFlags::default());
        parsed.templates.push((*target, content));
        content
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.parsed.borrow_mut().quirks = mode == QuirksMode::Quirks;
    }

    fn add_attrs_if_missing(&self, target: &usize, mut attrs: Vec<Attribute>) {
        if self.screen_style(&mut attrs) {
            self.parsed.borrow_mut().hidden_styles.push(*target);
        }
        self.inner.add_attrs_if_missing(target, attrs);
    }

    fn remove_from_parent(&self, target: &usize) {
        self.parsed.borrow_mut().removed.push(*target);
        self.inner.remove_from_parent(target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        self.inner.reparent_children(node, new_parent);
    }
}

fn opts(scripting: bool) -> ParseOpts {
    ParseOpts {
        tokenizer: TokenizerOpts::default(),
        tree_builder: TreeBuilderOpts {
            exact_errors: false,
            // Scripts run on a page's tree, so its scripting flag is set, as
            // in a browser: <noscript> content is raw text (HTML §13.2.6.4.4).
            // blitz-html parses with it off, where a `<script>` in a head
            // `<noscript>` ends the noscript and lands in the head. DOMParser's
            // documents have no browsing context, so scripting is off there.
            scripting_enabled: scripting,
            iframe_srcdoc: false,
            drop_doctype: false,
            quirks_mode: QuirksMode::NoQuirks,
        },
    }
}

/// Feeds `html` in chunks, stopping when `stop` says to.
fn feed<S>(mut parser: S, html: &str, stop: impl Fn() -> bool) -> (S::Output, bool)
where
    S: TendrilSink<html5ever::tendril::fmt::UTF8>,
{
    let mut at = 0;
    let mut stopped = false;
    while at < html.len() {
        if stop() {
            stopped = true;
            break;
        }
        let mut end = (at + CHUNK).min(html.len());
        while !html.is_char_boundary(end) {
            end += 1;
        }
        parser.process(StrTendril::from_slice(&html[at..end]));
        at = end;
    }
    (parser.finish(), stopped)
}

/// Parses `html` as the content of element `context` (HTML §13.4, as
/// html5ever does it) into the detached node `holder`: the parser's root
/// element ends up as `holder`'s only child, and the content under it.
/// `stop` is asked between chunks; `Parsed::over_limit` says if it stopped.
pub(crate) fn parse_fragment(
    mutr: &mut DocumentMutator<'_>,
    context: usize,
    holder: usize,
    html: &str,
    limits: ParseLimits,
    stop: impl Fn() -> bool,
) -> Parsed {
    let deepest = Rc::new(Cell::new(0u32));
    let created = Rc::new(Cell::new(0usize));
    let styles = Rc::new(RefCell::new(Vec::new()));
    let over_style = Rc::new(Cell::new(false));
    let sink = Sink {
        inner: DocumentHtmlParser::new(mutr),
        document: holder,
        parsed: RefCell::new(Parsed::default()),
        depths: RefCell::new(Vec::new()),
        deepest: deepest.clone(),
        created: created.clone(),
        styles: styles.clone(),
        over_style: over_style.clone(),
    };
    let parser = driver::parse_fragment_for_element(sink, opts(true), context, false, None);
    let (mut parsed, stopped) = feed(parser, html, || {
        stop()
            || over_style.get()
            || created.get() > limits.max_nodes
            || deepest.get() as usize > limits.max_depth
    });
    parsed.over_limit = stopped;
    parsed.over_style = over_style.get();
    stash_style_text(&mut *mutr, &styles.borrow(), &mut parsed.strings);
    reveal_styles(&mut *mutr, &parsed.hidden_styles);
    parsed
}

/// Parses a whole document under `document`, `mutr`'s document node or the
/// marker of another. `scripting` is the tree builder's scripting flag.
pub(crate) fn parse_document(
    mutr: &mut DocumentMutator<'_>,
    document: usize,
    html: &str,
    limits: ParseLimits,
    scripting: bool,
    stop: impl Fn() -> bool,
) -> Parsed {
    let deepest = Rc::new(Cell::new(0u32));
    let created = Rc::new(Cell::new(0usize));
    let styles = Rc::new(RefCell::new(Vec::new()));
    let over_style = Rc::new(Cell::new(false));
    let sink = Sink {
        document,
        inner: DocumentHtmlParser::new(mutr),
        parsed: RefCell::new(Parsed::default()),
        depths: RefCell::new(Vec::new()),
        deepest: deepest.clone(),
        created: created.clone(),
        styles: styles.clone(),
        over_style: over_style.clone(),
    };
    let parser = driver::parse_document(sink, opts(scripting));
    let (mut parsed, stopped) = feed(parser, html, || {
        stop()
            || over_style.get()
            || created.get() > limits.max_nodes
            || deepest.get() as usize > limits.max_depth
    });
    parsed.over_limit = stopped;
    parsed.over_style = over_style.get();
    // Before `mutr` flushes, which is when blitz-dom reads style text.
    stash_style_text(&mut *mutr, &styles.borrow(), &mut parsed.strings);
    reveal_styles(&mut *mutr, &parsed.hidden_styles);
    parsed
}

/// Gives the elements in `hidden` their `style` attributes back.
fn reveal_styles(mutr: &mut DocumentMutator<'_>, hidden: &[usize]) {
    for &id in hidden {
        if let Some(e) = mutr.doc.get_node_mut(id).and_then(|n| n.element_data_mut()) {
            style_attr::reveal(&mut e.attrs);
        }
    }
}

/// Moves the text under `styles` out of blitz-dom, into `out`.
///
/// blitz-dom parses a `<style>`'s text whenever the element or its children
/// change, attached or not, and panics on an `@import` whose URL can't be
/// resolved (`import.url.url().unwrap()`, net.rs). Which text does that can
/// only be told by parsing CSS, across node boundaries; so no style text
/// reaches it. `Document` keeps it in its side table and reads it from there;
/// blitz-dom sees empty text nodes, and builds an empty stylesheet from them.
fn stash_style_text(
    mutr: &mut DocumentMutator<'_>,
    styles: &[usize],
    out: &mut Vec<(usize, String)>,
) {
    let mut stack = styles.to_vec();
    while let Some(n) = stack.pop() {
        let Some(node) = mutr.doc.get_node_mut(n) else {
            continue;
        };
        if let Some(t) = node.text_data_mut()
            && !t.content.is_empty()
        {
            out.push((n, std::mem::take(&mut t.content)));
        }
        stack.extend(node.children.iter().copied());
    }
}
