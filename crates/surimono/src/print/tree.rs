//! The tree a printer's parser builds, and what can be written back from it.
//!
//! fulgur parses with blitz-html, which feeds html5ever into a blitz-dom
//! document. That document acts on each node as it joins (`style` attributes
//! and `<style>` text go to stylo, unresolvable URLs panic), so it can't be
//! built to look at it first. [`Tree::parse`] drives html5ever with the same
//! settings into a plain arena, and every decision that shapes the tree is
//! blitz-html's:
//!
//! - scripting off, so `<noscript>` holds markup;
//! - doctypes dropped, with the quirks mode still worked out;
//! - a template's contents are its children (`get_template_contents` answers
//!   with the template), which is why a printer reads `<style>` in one;
//! - no annotation-xml integration points (the hook keeps its default), so
//!   `<math><annotation-xml encoding=text/html><style>` holds markup;
//! - text appends to a text node just before it, and comments don't.
//!
//! The tests below compare this tree with blitz-html's own.

use std::cell::{Cell, Ref, RefCell};

use html5ever::driver::{self, ParseOpts};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::TokenizerOpts;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilderOpts, TreeSink};
use html5ever::{Attribute, LocalName, QualName, ns};

use super::Gate;
use crate::error::Error;

/// Bytes of HTML fed between looks at the limits and the gate. html5ever
/// checks each attribute against the tag's others, so cost grows with a tag's
/// length, but no one feed takes long: a tag that never ends is stopped here.
const CHUNK: usize = 4096;

/// What a parse may build, as [`surimono_dom::DomLimits`] has for a document.
#[derive(Debug, Clone, Copy)]
pub(super) struct TreeLimits {
    pub(super) max_nodes: usize,
    pub(super) max_depth: usize,
    pub(super) max_attributes: usize,
}

/// An element, as the parser made it.
#[derive(Debug)]
pub(crate) struct Element {
    pub(crate) name: QualName,
    pub(crate) attrs: Vec<Attribute>,
}

impl Element {
    /// Its name, in whatever namespace: blitz-dom goes by the local name.
    pub(crate) fn local(&self) -> &str {
        self.name.local.as_ref()
    }

    pub(crate) fn is_html(&self, local: &str) -> bool {
        self.name.ns == ns!(html) && self.local() == local
    }

    /// The first attribute with this local name.
    pub(crate) fn attr(&self, local: &str) -> Option<&str> {
        let found = self.attrs.iter().find(|a| a.name.local.as_ref() == local);
        found.map(|a| a.value.as_ref())
    }
}

#[derive(Debug)]
pub(crate) enum Data {
    Document,
    Element(Element),
    Text(String),
    /// Never written back, and no text to a `<style>`.
    Comment,
}

#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) parent: Option<usize>,
    pub(crate) children: Vec<usize>,
    pub(crate) data: Data,
    /// As placed, for the check while feeding; moves don't update it.
    depth: u32,
}

/// See the module docs. Node 0 is the document.
#[derive(Debug)]
pub(crate) struct Tree {
    pub(crate) nodes: Vec<Node>,
    quirks: bool,
    /// Answers for a handle that isn't an element, which html5ever never
    /// passes: its handles come from its stack of open elements.
    no_name: QualName,
}

impl Tree {
    fn new() -> Self {
        let document = Node {
            parent: None,
            children: Vec::new(),
            data: Data::Document,
            depth: 0,
        };
        Self {
            nodes: vec![document],
            quirks: false,
            no_name: QualName::new(None, ns!(html), LocalName::from("")),
        }
    }

    fn add(&mut self, data: Data) -> usize {
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data,
            depth: 0,
        });
        self.nodes.len() - 1
    }

    /// Links `child` under `parent` as blitz-dom does: added to the list with
    /// `add` first, then taken off its old parent's list by id.
    fn link(&mut self, parent: usize, child: usize, add: impl FnOnce(&mut Vec<usize>)) {
        add(&mut self.nodes[parent].children);
        let depth = self.nodes[parent].depth.saturating_add(1);
        let node = &mut self.nodes[child];
        node.depth = depth;
        if let Some(old) = node.parent.replace(parent) {
            self.nodes[old].children.retain(|c| *c != child);
        }
    }

    fn append_text(&mut self, node: Option<usize>, text: &str) -> bool {
        let last = node.map(|n| &mut self.nodes[n].data);
        if let Some(Data::Text(t)) = last {
            t.push_str(text);
            return true;
        }
        false
    }

    /// Takes `id` and what's under it out of the tree.
    pub(crate) fn detach(&mut self, id: usize) {
        if let Some(parent) = self.nodes[id].parent.take() {
            self.nodes[parent].children.retain(|c| *c != id);
        }
    }

    /// `id`'s only child becomes a text node holding `text`.
    pub(crate) fn set_text(&mut self, id: usize, text: String) {
        for child in std::mem::take(&mut self.nodes[id].children) {
            self.nodes[child].parent = None;
        }
        let new = self.add(Data::Text(text));
        self.link(id, new, |c| c.push(new));
    }

    /// The text under `id`, at any depth, as blitz-dom's `text_content`.
    pub(crate) fn text_of(&self, id: usize) -> String {
        let mut out = String::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            match &node.data {
                Data::Text(t) => out.push_str(t),
                Data::Element(_) => stack.extend(node.children.iter().rev().copied()),
                Data::Document | Data::Comment => {}
            }
        }
        out
    }

    /// Every element named `local` in any namespace, in tree order.
    pub(crate) fn elements_named(&self, local: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut stack = vec![0];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            if let Data::Element(e) = &node.data
                && e.local() == local
            {
                found.push(n);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        found
    }

    pub(crate) fn element(&self, id: usize) -> Option<&Element> {
        match &self.nodes[id].data {
            Data::Element(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn element_mut(&mut self, id: usize) -> Option<&mut Element> {
        match &mut self.nodes[id].data {
            Data::Element(e) => Some(e),
            _ => None,
        }
    }

    /// Parses `html` as a printer does; see the module docs.
    pub(super) fn parse(html: &str, limits: TreeLimits, gate: &Gate<'_>) -> Result<Self, Error> {
        let watch = Watch::default();
        let sink = Sink {
            tree: RefCell::new(Self::new()),
            watch: &watch,
        };
        let mut parser = driver::parse_document(sink, printer_opts());
        let mut at = 0;
        while at < html.len() {
            gate.check()?;
            watch.check(limits)?;
            let mut end = (at + CHUNK).min(html.len());
            while !html.is_char_boundary(end) {
                end += 1;
            }
            parser.process(StrTendril::from_slice(&html[at..end]));
            at = end;
        }
        let tree = parser.finish();
        gate.check()?;
        watch.check(limits)?;
        Ok(tree)
    }

    /// Writes the tree as HTML that parses back to it: HTML §13.3 for scripting
    /// off, without comments, with what §13.3 leaves to a reparse put right.
    ///
    /// - A newline starting a `<pre>`, `<textarea>` or `<listing>` is doubled,
    ///   because the parser drops the first.
    /// - A carriage return is written as a reference, because a parser turns a
    ///   literal one into a newline.
    /// - `<plaintext>` ends the document: nothing can close it, so its text is
    ///   the last thing written.
    pub(super) fn to_html(&self, max_bytes: usize, gate: &Gate<'_>) -> Result<String, Error> {
        enum Step {
            Open(usize),
            Close(usize),
        }
        let mut out = String::new();
        if !self.quirks {
            out.push_str("<!doctype html>");
        }
        let mut stack: Vec<Step> = self.nodes[0]
            .children
            .iter()
            .rev()
            .map(|&c| Step::Open(c))
            .collect();
        while let Some(step) = stack.pop() {
            gate.check()?;
            if out.len() > max_bytes {
                return Err(too_long(max_bytes));
            }
            let id = match step {
                Step::Close(id) => {
                    if let Some(e) = self.element(id) {
                        if e.is_html("plaintext") {
                            break;
                        }
                        out.push_str("</");
                        out.push_str(e.local());
                        out.push('>');
                    }
                    continue;
                }
                Step::Open(id) => id,
            };
            let node = &self.nodes[id];
            match &node.data {
                Data::Document | Data::Comment => {}
                Data::Text(text) => {
                    if out.len().saturating_add(text.len()) > max_bytes {
                        return Err(too_long(max_bytes));
                    }
                    let parent = node.parent.and_then(|p| self.element(p));
                    if parent.is_some_and(holds_raw_text) {
                        out.push_str(text);
                    } else {
                        escape_into(text, false, &mut out);
                    }
                }
                Data::Element(e) => {
                    out.push('<');
                    out.push_str(e.local());
                    for a in &e.attrs {
                        out.push(' ');
                        if let Some(prefix) = &a.name.prefix {
                            out.push_str(prefix);
                            out.push(':');
                        }
                        out.push_str(a.name.local.as_ref());
                        out.push_str("=\"");
                        escape_into(&a.value, true, &mut out);
                        out.push('"');
                    }
                    out.push('>');
                    if is_void(e) {
                        continue;
                    }
                    if drops_first_newline(e)
                        && let Some(&first) = node.children.first()
                        && let Data::Text(t) = &self.nodes[first].data
                        && t.starts_with('\n')
                    {
                        out.push('\n');
                    }
                    stack.push(Step::Close(id));
                    stack.extend(node.children.iter().rev().map(|&c| Step::Open(c)));
                }
            }
        }
        if out.len() > max_bytes {
            return Err(too_long(max_bytes));
        }
        Ok(out)
    }
}

/// The error for a page past [`Limits::output_bytes`](crate::Limits::output_bytes).
pub(super) fn too_long(max_bytes: usize) -> Error {
    Error::PageDomLimit {
        message: format!("the page's HTML is over the output limit of {max_bytes} bytes"),
    }
}

/// The settings of blitz-html 0.2.0's `parse_into_mutator`.
fn printer_opts() -> ParseOpts {
    ParseOpts {
        tokenizer: TokenizerOpts::default(),
        tree_builder: TreeBuilderOpts {
            exact_errors: false,
            scripting_enabled: false,
            iframe_srcdoc: false,
            drop_doctype: true,
            quirks_mode: QuirksMode::NoQuirks,
        },
    }
}

/// HTML §13.3's list, without `noscript`: scripting is off.
fn holds_raw_text(e: &Element) -> bool {
    e.name.ns == ns!(html)
        && matches!(
            e.local(),
            "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext"
        )
}

fn is_void(e: &Element) -> bool {
    e.name.ns == ns!(html)
        && matches!(
            e.local(),
            "area"
                | "base"
                | "basefont"
                | "bgsound"
                | "br"
                | "col"
                | "embed"
                | "frame"
                | "hr"
                | "img"
                | "input"
                | "keygen"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
}

fn drops_first_newline(e: &Element) -> bool {
    e.name.ns == ns!(html) && matches!(e.local(), "pre" | "textarea" | "listing")
}

fn escape_into(text: &str, attribute: bool, out: &mut String) {
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let with = match c {
            '&' => "&amp;",
            '\u{a0}' => "&nbsp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '\r' => "&#13;",
            '"' if attribute => "&quot;",
            _ => continue,
        };
        out.push_str(&text[start..i]);
        out.push_str(with);
        start = i + c.len_utf8();
    }
    out.push_str(&text[start..]);
}

/// How the parse is going, for the loop that feeds it.
#[derive(Default)]
struct Watch {
    created: Cell<usize>,
    deepest: Cell<u32>,
    attributes: Cell<usize>,
}

impl Watch {
    fn check(&self, limits: TreeLimits) -> Result<(), Error> {
        let message = if self.created.get() > limits.max_nodes {
            format!("the page would parse to over {} nodes", limits.max_nodes)
        } else if self.deepest.get() as usize > limits.max_depth {
            format!("the page would nest over {} levels", limits.max_depth)
        } else if self.attributes.get() > limits.max_attributes {
            format!(
                "an element would have over {} attributes",
                limits.max_attributes
            )
        } else {
            return Ok(());
        };
        Err(Error::PageDomLimit { message })
    }
}

/// blitz-html's sink, with an arena in place of the document.
struct Sink<'w> {
    tree: RefCell<Tree>,
    watch: &'w Watch,
}

impl Sink<'_> {
    fn note_attributes(&self, count: usize) {
        self.watch
            .attributes
            .set(self.watch.attributes.get().max(count));
    }

    fn created(&self) {
        self.watch.created.set(self.watch.created.get() + 1);
    }

    fn placed(&self, tree: &Tree, id: usize) {
        let deepest = self.watch.deepest.get().max(tree.nodes[id].depth);
        self.watch.deepest.set(deepest);
    }
}

impl TreeSink for Sink<'_> {
    type Output = Tree;
    type Handle = usize;
    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    fn finish(self) -> Tree {
        self.tree.into_inner()
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.tree.borrow(), |tree| {
            tree.element(*target).map_or(&tree.no_name, |e| &e.name)
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, _: ElementFlags) -> usize {
        self.created();
        self.note_attributes(attrs.len());
        self.tree
            .borrow_mut()
            .add(Data::Element(Element { name, attrs }))
    }

    fn create_comment(&self, _text: StrTendril) -> usize {
        self.created();
        self.tree.borrow_mut().add(Data::Comment)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> usize {
        self.created();
        self.tree.borrow_mut().add(Data::Comment)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        let mut tree = self.tree.borrow_mut();
        let id = match child {
            NodeOrText::AppendNode(id) => id,
            NodeOrText::AppendText(text) => {
                let last = tree.nodes[*parent].children.last().copied();
                if tree.append_text(last, &text) {
                    return;
                }
                tree.add(Data::Text(text.to_string()))
            }
        };
        tree.link(*parent, id, |c| c.push(id));
        self.placed(&tree, id);
    }

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let mut tree = self.tree.borrow_mut();
        let Some(parent) = tree.nodes[*sibling].parent else {
            return;
        };
        let at = tree.nodes[parent]
            .children
            .iter()
            .position(|c| c == sibling);
        let at = at.unwrap_or(tree.nodes[parent].children.len());
        let id = match new_node {
            NodeOrText::AppendNode(id) => id,
            NodeOrText::AppendText(text) => {
                let before = at.checked_sub(1).map(|i| tree.nodes[parent].children[i]);
                if tree.append_text(before, &text) {
                    return;
                }
                tree.add(Data::Text(text.to_string()))
            }
        };
        tree.link(parent, id, |c| c.insert(at, id));
        self.placed(&tree, id);
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        let has_parent = self.tree.borrow().nodes[*element].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    /// As blitz-html: the template itself.
    fn get_template_contents(&self, target: &usize) -> usize {
        *target
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.tree.borrow_mut().quirks = mode == QuirksMode::Quirks;
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        let mut tree = self.tree.borrow_mut();
        let Some(element) = tree.element_mut(*target) else {
            return;
        };
        for attr in attrs {
            if !element.attrs.iter().any(|a| a.name == attr.name) {
                element.attrs.push(attr);
            }
        }
        let count = element.attrs.len();
        self.note_attributes(count);
    }

    fn remove_from_parent(&self, target: &usize) {
        self.tree.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        let mut tree = self.tree.borrow_mut();
        let children = std::mem::take(&mut tree.nodes[*node].children);
        for child in children {
            tree.link(*new_parent, child, |c| c.push(child));
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::fmt::Write;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use blitz_dom::{BaseDocument, DocumentConfig, NodeData};
    use blitz_html::HtmlDocument;

    use super::*;

    const WIDE: TreeLimits = TreeLimits {
        max_nodes: usize::MAX,
        max_depth: usize::MAX,
        max_attributes: usize::MAX,
    };

    fn attr_line(out: &mut String, prefix: &str, local: &str, value: &str) {
        let _ = writeln!(out, "  @{prefix}:{local}={value:?}");
    }

    /// One line per node, sorted attributes, for either tree.
    fn dump_ours(tree: &Tree) -> String {
        let mut out = String::new();
        let mut stack = vec![(0, 0)];
        while let Some((id, depth)) = stack.pop() {
            let node = &tree.nodes[id];
            let _ = write!(out, "{depth} ");
            match &node.data {
                Data::Document => out.push_str("document\n"),
                Data::Comment => out.push_str("comment\n"),
                Data::Text(t) => {
                    let _ = writeln!(out, "{t:?}");
                }
                Data::Element(e) => {
                    let _ = writeln!(out, "{:?} {}", e.name.ns, e.name.local);
                    let mut attrs: Vec<_> = e.attrs.iter().collect();
                    attrs.sort_by_key(|a| (a.name.prefix.clone(), a.name.local.clone()));
                    for a in attrs {
                        let prefix = a.name.prefix.as_deref().unwrap_or("");
                        attr_line(&mut out, prefix, &a.name.local, &a.value);
                    }
                }
            }
            stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
        }
        out
    }

    fn dump_blitz(doc: &BaseDocument) -> String {
        let mut out = String::new();
        let mut stack = vec![(0, 0)];
        while let Some((id, depth)) = stack.pop() {
            let node = doc.get_node(id).unwrap_or_else(|| panic!("node {id}"));
            let _ = write!(out, "{depth} ");
            match &node.data {
                NodeData::Document => out.push_str("document\n"),
                NodeData::Comment => out.push_str("comment\n"),
                NodeData::Text(t) => {
                    let _ = writeln!(out, "{:?}", t.content);
                }
                NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                    let _ = writeln!(out, "{:?} {}", e.name.ns, e.name.local);
                    let mut attrs: Vec<_> = e.attrs().iter().collect();
                    attrs.sort_by_key(|a| (a.name.prefix.clone(), a.name.local.clone()));
                    for a in attrs {
                        let prefix = a.name.prefix.as_deref().unwrap_or("");
                        attr_line(&mut out, prefix, &a.name.local, &a.value);
                    }
                }
            }
            stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
        }
        out
    }

    fn ours(html: &str) -> Tree {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(60));
        Tree::parse(html, WIDE, &gate).unwrap_or_else(|e| panic!("{e}"))
    }

    fn assert_same_tree(html: &str) {
        let config = DocumentConfig {
            base_url: Some("file:///".to_owned()),
            ..DocumentConfig::default()
        };
        let doc = HtmlDocument::from_html(html, config);
        assert_eq!(dump_ours(&ours(html)), dump_blitz(&doc), "{html}");
    }

    #[test]
    fn the_tree_is_blitz_htmls() {
        for html in [
            "<!doctype html><title>t</title><p>x",
            "<p>no doctype",
            "<!doctype html><template><noscript></template><p>x</p></noscript>y",
            "<!doctype html><head><noscript><link rel=x><meta name=y></noscript></head>",
            "<!doctype html><svg><template><p></template></svg>",
            "<!doctype html><svg><style>a&amp;b</style><title><p>x</p></title></svg>",
            "<!doctype html><svg viewBox=\"0 0 1 1\" xlink:href=\"a\" xml:lang=\"en\"><foreignObject><p>x</foreignObject></svg>",
            "<!doctype html><math><annotation-xml encoding=text/html><style><b>x</b></style></annotation-xml></math>",
            "<!doctype html><math><mtext><p>x</mtext><mi><style>y</style></mi></math>",
            "<!doctype html><table><tr><td>a<table>b</table>c<tr>d</table>e",
            "<!doctype html><table>x<b>y<td>z</b></table>",
            "<!doctype html><b><p>x</b>y</p><a href=1>a<a href=2>b",
            "<!doctype html><a><table><a>x</a></table>",
            "<!doctype html><b><i><u><p>a</b>b</i>c</u>d</p>",
            "<!doctype html><select><option>a<optgroup><xmp>b</xmp></select>",
            "<!doctype html><form><div></form><form>x</form>",
            "<!doctype html><html a=1><body b=2><html c=3 a=4><body d=5 b=6>",
            "<!doctype html><frameset><frame><noframes>x</noframes></frameset>",
            "<!doctype html><pre>\n\nx</pre><textarea>\n\ny</textarea><listing>\nz</listing>",
            "<!doctype html><plaintext>x</plaintext><b>y",
            "<!doctype html><script><!--<script>x</script>y--></script>z",
            "<!doctype html><style>a</style</style>b",
            "<!doctype html><p a<b=c \"=d =e>x</p>",
            "<!doctype html><br/></br></p><image src=a><isindex>",
            "<!doctype html><template><tr><td>a</template><template><col></template>",
            "<!doctype html><template shadowrootmode=open><p>x</p></template>",
            "<?pi x><!doctype html><!--a--><html><!--b--><head><!--c-->x<!--d-->y",
            "<table><p>quirks keeps p open<table>",
        ] {
            assert_same_tree(html);
        }
    }

    fn step(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    /// A page of markup likely to confuse parsers, from `seed`.
    pub(in crate::print) fn scrambled(seed: &mut u64) -> String {
        const PIECES: &[&str] = &[
            "<table>",
            "</table>",
            "<tr>",
            "<td>",
            "<caption>",
            "<select>",
            "<option>",
            "<a>",
            "<b>",
            "</b>",
            "<i>",
            "<p>",
            "</p>",
            "<div>",
            "</div>",
            "<svg>",
            "</svg>",
            "<math>",
            "</math>",
            "<mtext>",
            "<mi>",
            "<annotation-xml encoding=text/html>",
            "</annotation-xml>",
            "<foreignObject>",
            "</foreignObject>",
            "<desc>",
            "<title>",
            "</title>",
            "<template>",
            "</template>",
            "<noscript>",
            "</noscript>",
            "<style>",
            "</style>",
            "<script>",
            "</script>",
            "<textarea>",
            "</textarea>",
            "<xmp>",
            "</xmp>",
            "<iframe>",
            "</iframe>",
            "<noembed>",
            "</noembed>",
            "<noframes>",
            "</noframes>",
            "<plaintext>",
            "<frameset>",
            "<form>",
            "</form>",
            "<!--",
            "-->",
            "--!>",
            "<!",
            "<?",
            "<head>",
            "<body>",
            "</body>",
            "<html>",
            "</html>",
            "<pre>\n\n",
            "<br>",
            "</br>",
            "<image src=a>",
            "<img src=a>",
            "text",
            " ",
            "&amp;",
            "&lt;script&gt;",
            "\"",
            "'",
            ">",
            "<",
            "</",
            "\n",
            "&#13;",
        ];
        let mut html = String::from(if step(seed) % 2 == 0 {
            "<!doctype html>"
        } else {
            ""
        });
        for _ in 0..(step(seed) % 40) {
            html.push_str(PIECES[(step(seed) % PIECES.len() as u64) as usize]);
        }
        html
    }

    #[test]
    fn scrambled_markup_makes_the_same_tree() {
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..1_500 {
            assert_same_tree(&scrambled(&mut seed));
        }
    }
}
