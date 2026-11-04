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
//! - style text: see [`stash_style_text`].

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

/// Bytes of HTML fed between limit checks.
const CHUNK: usize = 4096;

/// What the sink learned, beyond the tree.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// (node, data): comment data, and the text of `<style>` elements.
    pub strings: Vec<(usize, String)>,
    /// Nodes the tree builder took out of the tree. It puts most back; what
    /// has no parent at the end (`<frameset>` drops the `<body>`) is an
    /// orphan for the caller to free.
    pub removed: Vec<usize>,
    pub quirks: bool,
    /// Feeding stopped because the tree went over a limit.
    pub over_limit: bool,
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
}

impl Sink<'_, '_> {
    fn note_created(&self) {
        self.created.set(self.created.get() + 1);
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

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        self.note_created();
        let is_style = name.local.as_ref() == "style";
        let id = self.inner.create_element(name, attrs, flags);
        if is_style {
            self.styles.borrow_mut().push(id);
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

    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        self.inner.append_doctype_to_document(name, public, system);
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        self.inner.get_template_contents(target)
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.parsed.borrow_mut().quirks = mode == QuirksMode::Quirks;
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
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

fn opts() -> ParseOpts {
    ParseOpts {
        tokenizer: TokenizerOpts::default(),
        tree_builder: TreeBuilderOpts {
            exact_errors: false,
            // Scripts run on this tree, so its scripting flag is set, as in
            // a browser: <noscript> content is raw text (HTML §13.2.6.4.4).
            // blitz-html parses with it off, where a `<script>` in a head
            // `<noscript>` ends the noscript and lands in the head.
            scripting_enabled: true,
            iframe_srcdoc: false,
            drop_doctype: true,
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
    let sink = Sink {
        inner: DocumentHtmlParser::new(mutr),
        document: holder,
        parsed: RefCell::new(Parsed::default()),
        depths: RefCell::new(Vec::new()),
        deepest: deepest.clone(),
        created: created.clone(),
        styles: styles.clone(),
    };
    let parser = driver::parse_fragment_for_element(sink, opts(), context, false, None);
    let (mut parsed, stopped) = feed(parser, html, || {
        stop() || created.get() > limits.max_nodes || deepest.get() as usize > limits.max_depth
    });
    parsed.over_limit = stopped;
    stash_style_text(&mut *mutr, &styles.borrow(), &mut parsed.strings);
    parsed
}

/// Parses a whole document into `mutr`'s empty document.
pub(crate) fn parse_document(
    mutr: &mut DocumentMutator<'_>,
    html: &str,
    limits: ParseLimits,
) -> Parsed {
    let deepest = Rc::new(Cell::new(0u32));
    let created = Rc::new(Cell::new(0usize));
    let styles = Rc::new(RefCell::new(Vec::new()));
    let sink = Sink {
        document: 0,
        inner: DocumentHtmlParser::new(mutr),
        parsed: RefCell::new(Parsed::default()),
        depths: RefCell::new(Vec::new()),
        deepest: deepest.clone(),
        created: created.clone(),
        styles: styles.clone(),
    };
    let parser = driver::parse_document(sink, opts());
    let (mut parsed, stopped) = feed(parser, html, || {
        created.get() > limits.max_nodes || deepest.get() as usize > limits.max_depth
    });
    parsed.over_limit = stopped;
    // Before `mutr` flushes, which is when blitz-dom reads style text.
    stash_style_text(&mut *mutr, &styles.borrow(), &mut parsed.strings);
    parsed
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
