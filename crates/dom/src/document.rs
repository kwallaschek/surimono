//! A parsed document and the handle table that scripts address it through.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use blitz_dom::node::NodeFlags;
use blitz_dom::{BaseDocument, DocumentConfig, LocalName, NodeData, QualName, ns};

use crate::DomError;
use crate::parse::{self, ParseLimits};

/// Generations are unique in the process, so a wrapper can never match a
/// document other than its own. Starts at 1; 0 is never a generation.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Limits on the Rust-side document a script can grow. The JavaScript heap
/// limit doesn't cover nodes, which live in Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomLimits {
    /// Most nodes the document may hold (parsed and created together).
    /// Checked when parsing and on every node a script creates.
    pub max_nodes: usize,
    /// Deepest nesting, counted from the document node (the `<html>`
    /// element is at depth 1). Checked when parsing and on every insertion.
    /// Bounds the recursion in blitz-dom's subtree walks (adding a subtree
    /// to the document), which could otherwise overflow the stack; at most
    /// [`MAX_DEPTH`](Self::MAX_DEPTH).
    pub max_depth: usize,
    /// Most handles the document may give out over its life. A handle is
    /// never reused, so its table entry outlives its node; this bounds that
    /// table (16 bytes a handle, outside the JavaScript heap limit) when a
    /// script creates and drops nodes in a loop.
    pub max_handles: usize,
    /// Most bytes of strings the document may hold: text, comment data, and
    /// attribute names and values (with [`ATTRIBUTE_COST`] each), parsed and
    /// from scripts together. Counted on write and credited on overwrite and
    /// on drop. These strings live in Rust, outside the JavaScript heap
    /// limit, and one JavaScript string can be stored on many nodes.
    pub max_text_bytes: usize,
}

/// Bytes charged per attribute beyond its name and value: what the entry
/// itself takes, so empty attributes aren't free.
pub const ATTRIBUTE_COST: usize = 64;

impl DomLimits {
    /// 100,000 nodes: a large report page has a few thousand.
    pub const DEFAULT_MAX_NODES: usize = 100_000;
    /// 512 levels. Chrome renders deeper trees, but no real page nests
    /// this deep.
    pub const DEFAULT_MAX_DEPTH: usize = 512;
    /// Largest `max_depth` accepted: blitz-dom recurses once per level when
    /// a subtree joins the document, and 1,024 levels fit the sandbox's
    /// stack headroom (tested on a 1 MiB thread).
    pub const MAX_DEPTH: usize = 1_024;
    /// 1,000,000 handles (16 MB of table at most).
    pub const DEFAULT_MAX_HANDLES: usize = 1_000_000;
    /// 256 MiB: far above a real page, inline images included, and small
    /// enough that a script hits this rather than the process's memory.
    pub const DEFAULT_MAX_TEXT_BYTES: usize = 256 * 1024 * 1024;

    /// Checks the limits are in range: every limit at least 1, `max_depth`
    /// at most [`MAX_DEPTH`](Self::MAX_DEPTH), `max_handles` within `u32`.
    pub fn validate(&self) -> Result<(), DomError> {
        let bad = |what: &str| {
            Err(DomError::InvalidLimits {
                message: what.to_owned(),
            })
        };
        if self.max_nodes == 0
            || self.max_depth == 0
            || self.max_handles == 0
            || self.max_text_bytes == 0
        {
            return bad("every DOM limit must be at least 1");
        }
        if self.max_depth > Self::MAX_DEPTH {
            return bad("max_depth must be at most DomLimits::MAX_DEPTH (1,024)");
        }
        if u32::try_from(self.max_handles).is_err() {
            return bad("max_handles must fit in 32 bits");
        }
        Ok(())
    }
}

impl Default for DomLimits {
    fn default() -> Self {
        Self {
            max_nodes: Self::DEFAULT_MAX_NODES,
            max_depth: Self::DEFAULT_MAX_DEPTH,
            max_handles: Self::DEFAULT_MAX_HANDLES,
            max_text_bytes: Self::DEFAULT_MAX_TEXT_BYTES,
        }
    }
}

/// What kind of node a handle names, as the DOM's `nodeType` numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Element = 1,
    Text = 3,
    Comment = 8,
    Document = 9,
}

/// A handle as JavaScript holds it: the document's generation and a handle
/// number. Neither is a pointer or a blitz node id.
pub(crate) type Handle = u32;

/// A document scripts can run against: a blitz-dom document plus the table
/// that maps the handles given to JavaScript to blitz node ids.
///
/// Handle numbers are never reused within a document, while blitz-dom's
/// node ids (slab slots) are; so a wrapper for a dropped node can't resolve
/// to whichever node reuses its slot.
pub struct Document {
    inner: BaseDocument,
    generation: u64,
    limits: DomLimits,
    /// Handle (1-based) minus one to blitz node id; `None` once the node is
    /// dropped.
    handle_to_node: Vec<Option<usize>>,
    /// Blitz node id to its handle, 0 for none yet.
    node_to_handle: Vec<Handle>,
    /// Strings blitz-dom doesn't hold, by node id: comment data
    /// (`NodeData::Comment` has no room for it) and text under a `<style>`
    /// (see [`parse`](crate::parse)). Only looked up, so hash order can't
    /// reach any output.
    side_data: HashMap<usize, String>,
    /// Bytes held (see [`DomLimits::max_text_bytes`]).
    held_bytes: usize,
    /// The document was parsed in quirks mode.
    quirks: bool,
    /// The last (parent, index in its children) a sibling step found, so
    /// walking a wide parent's children isn't quadratic. Checked on use.
    cursor: Cell<(usize, usize)>,
    /// A fatal error a binding met, for the native call to report.
    pub(crate) fault: Option<DomError>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("generation", &self.generation)
            .field("nodes", &self.inner.tree().len())
            .field("handles", &self.handle_to_node.len())
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Parses `html` with the default [`DomLimits`].
    pub fn parse(html: &str) -> Result<Self, DomError> {
        Self::parse_with_limits(html, DomLimits::default())
    }

    /// Parses `html`. No network, no system fonts: blitz-dom is built
    /// without them and the config has no providers.
    ///
    /// Fails with [`DomError::NodeLimit`] or [`DomError::TreeTooDeep`] if
    /// the parsed tree is already over `limits` (html5ever doesn't cap
    /// nesting), and with [`DomError::InvalidLimits`] for limits out of
    /// range. The parser itself doesn't recurse per level, and a refused
    /// tree is freed without recursing (blitz-dom keeps nodes in a slab).
    pub fn parse_with_limits(html: &str, limits: DomLimits) -> Result<Self, DomError> {
        limits.validate()?;
        let mut inner = BaseDocument::new(DocumentConfig::default());
        // Nothing is "in the document" as far as blitz-dom can tell. Its
        // hooks for elements joining the document have no use here (nothing
        // loads, nothing lays out yet) and do harm: they panic on URLs they
        // can't resolve, and add text children to <input> outside `max_nodes`.
        inner
            .root_node_mut()
            .flags
            .remove(NodeFlags::IS_IN_DOCUMENT);
        let parsed = parse::parse_document(
            &mut inner.mutate(),
            html,
            ParseLimits {
                max_nodes: limits.max_nodes,
                max_depth: limits.max_depth,
            },
        );
        let mut doc = Self {
            inner,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            limits,
            handle_to_node: Vec::new(),
            node_to_handle: Vec::new(),
            side_data: parsed.strings.into_iter().collect(),
            held_bytes: 0,
            quirks: parsed.quirks,
            cursor: Cell::new((0, 0)),
            fault: None,
        };
        if doc.node_count() > limits.max_nodes {
            return Err(DomError::NodeLimit {
                limit: limits.max_nodes,
            });
        }
        if doc.height(doc.root()) - 1 > limits.max_depth || parsed.over_limit {
            return Err(DomError::TreeTooDeep {
                limit: limits.max_depth,
            });
        }
        doc.held_bytes = doc.bytes_under(doc.root());
        if doc.held_bytes > limits.max_text_bytes {
            return Err(DomError::TextLimit {
                limit: limits.max_text_bytes,
            });
        }
        Ok(doc)
    }

    /// The document's serialization: the `<html>` element's outer HTML, by
    /// the HTML Standard's fragment serialization algorithm (§13.3) for
    /// elements (HTML, SVG and MathML) with their attributes, text and
    /// comments, with scripting disabled, as blitz-html parsed the
    /// document. Iterative, so tree depth can't overflow the stack.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        if let Some(id) = self.document_element() {
            self.serialize(id, &mut out);
        }
        out
    }

    fn serialize(&self, top: usize, out: &mut String) {
        enum Step {
            Open(usize),
            Close(usize),
        }
        let mut stack = vec![Step::Open(top)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Close(id) => {
                    if let Some(e) = self.node(id).element_data() {
                        out.push_str("</");
                        out.push_str(&e.name.local);
                        out.push('>');
                    }
                }
                Step::Open(id) => {
                    let node = self.node(id);
                    match &node.data {
                        NodeData::Text(_) => {
                            // §13.3: text is written unescaped when its
                            // parent is an HTML style, script, xmp, iframe,
                            // noembed, noframes or plaintext element (an SVG
                            // <style> or <script> is escaped), or a noscript
                            // element when scripting is enabled for the
                            // node. It isn't here: blitz-html 0.2.0 parses
                            // with `scripting_enabled: false`, so noscript
                            // content is parsed as markup and its text must
                            // be escaped to survive a reparse. Written raw,
                            // `<noscript>&lt;b&gt;</noscript>` would come
                            // back as a real <b> element.
                            let raw = node
                                .parent
                                .and_then(|p| self.node(p).element_data())
                                .is_some_and(|e| {
                                    e.name.ns == ns!(html)
                                        && matches!(
                                            e.name.local.as_ref(),
                                            "style"
                                                | "script"
                                                | "xmp"
                                                | "iframe"
                                                | "noembed"
                                                | "noframes"
                                                | "plaintext"
                                        )
                                });
                            if raw {
                                out.push_str(self.char_data(id));
                            } else {
                                escape_into(self.char_data(id), false, out);
                            }
                        }
                        // §13.3: comment data goes out unescaped.
                        NodeData::Comment => {
                            out.push_str("<!--");
                            out.push_str(self.char_data(id));
                            out.push_str("-->");
                        }
                        NodeData::Document => {}
                        NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                            out.push('<');
                            out.push_str(&e.name.local);
                            for a in e.attrs() {
                                out.push(' ');
                                push_attribute_name(&a.name, out);
                                out.push_str("=\"");
                                escape_into(&a.value, true, out);
                                out.push('"');
                            }
                            out.push('>');
                            let void = e.name.ns == ns!(html)
                                && matches!(
                                    e.name.local.as_ref(),
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
                                );
                            if !void {
                                stack.push(Step::Close(id));
                                stack.extend(node.children.iter().rev().map(|&c| Step::Open(c)));
                            }
                        }
                    }
                }
            }
        }
    }

    /// The live node count, parsed and created.
    pub fn node_count(&self) -> usize {
        self.inner.tree().len()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn node(&self, id: usize) -> &blitz_dom::Node {
        // Ids come from `resolve`, the tree's own links or blitz-dom's id
        // map; all name live nodes because every drop goes through
        // `drop_children`, which updates the table first.
        &self.inner.tree()[id]
    }

    /// The handle for blitz node `id`, made on first use. Fails with
    /// [`DomError::HandleLimit`] once the document has given out
    /// `max_handles`.
    pub(crate) fn handle_for(&mut self, id: usize) -> Result<Handle, DomError> {
        if self.node_to_handle.len() <= id {
            self.node_to_handle.resize(id + 1, 0);
        }
        if self.node_to_handle[id] == 0 {
            if self.handle_to_node.len() >= self.limits.max_handles {
                return Err(DomError::HandleLimit {
                    limit: self.limits.max_handles,
                });
            }
            self.handle_to_node.push(Some(id));
            // `validate` keeps max_handles within u32, so this always fits;
            // 0 (never a valid handle, resolves to NodeRemoved) otherwise.
            self.node_to_handle[id] = Handle::try_from(self.handle_to_node.len()).unwrap_or(0);
        }
        Ok(self.node_to_handle[id])
    }

    /// Checks `(generation, handle)` names a node.
    pub(crate) fn check(&self, generation: u64, handle: Handle) -> Result<(), DomError> {
        self.resolve(generation, handle).map(|_| ())
    }

    /// The blitz node id that `(generation, handle)` names.
    pub(crate) fn resolve(&self, generation: u64, handle: Handle) -> Result<usize, DomError> {
        if generation != self.generation {
            return Err(DomError::StaleDocument);
        }
        match handle
            .checked_sub(1)
            .and_then(|i| self.handle_to_node.get(i as usize))
        {
            Some(Some(id)) => Ok(*id),
            // Handles only come from `handle_for`, so an unknown one is a
            // dropped node's.
            _ => Err(DomError::NodeRemoved),
        }
    }

    pub(crate) fn kind(&self, id: usize) -> Option<Kind> {
        match self.node(id).data {
            NodeData::Document => Some(Kind::Document),
            NodeData::Element(_) => Some(Kind::Element),
            NodeData::Text(_) => Some(Kind::Text),
            NodeData::Comment => Some(Kind::Comment),
            // Layout-only boxes; never reachable from the tree links scripts
            // follow before layout.
            NodeData::AnonymousBlock(_) => None,
        }
    }

    /// The document node's id.
    pub(crate) fn root(&self) -> usize {
        self.inner.root_node().id
    }

    pub(crate) fn document_element(&self) -> Option<usize> {
        self.inner.try_root_element().map(|n| n.id)
    }

    /// The first HTML child of `<html>` with this local name.
    fn html_child(&self, local: &str) -> Option<usize> {
        let html = self.document_element()?;
        self.node(html).children.iter().copied().find(|&c| {
            self.node(c)
                .element_data()
                .is_some_and(|e| e.name.ns == ns!(html) && e.name.local.as_ref() == local)
        })
    }

    pub(crate) fn head(&self) -> Option<usize> {
        self.html_child("head")
    }

    pub(crate) fn body(&self) -> Option<usize> {
        self.html_child("body")
    }

    /// The first element in tree order with this ID. blitz-dom's own map
    /// keeps one node per ID and misses IDs set after insertion, so this
    /// walks the tree.
    pub(crate) fn element_by_id(&self, id: &str) -> Option<usize> {
        if id.is_empty() {
            return None;
        }
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if node
                .element_data()
                .is_some_and(|e| e.attr(blitz_dom::local_name!("id")) == Some(id))
            {
                return Some(n);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        None
    }

    pub(crate) fn parent(&self, id: usize) -> Option<usize> {
        self.node(id).parent
    }

    pub(crate) fn children(&self, id: usize) -> &[usize] {
        &self.node(id).children
    }

    pub(crate) fn is_quirks(&self) -> bool {
        self.quirks
    }

    /// The sibling `offset` (1 or -1) places from `id`.
    pub(crate) fn sibling(&self, id: usize, forward: bool) -> Option<usize> {
        let parent = self.node(id).parent?;
        let list = &self.node(parent).children;
        let (p, i) = self.cursor.get();
        let at = if p == parent && list.get(i) == Some(&id) {
            i
        } else {
            list.iter().position(|&c| c == id)?
        };
        let next = if forward {
            at.checked_add(1)?
        } else {
            at.checked_sub(1)?
        };
        let found = *list.get(next)?;
        self.cursor.set((parent, next));
        Some(found)
    }

    /// Whether `id`'s tree is the document's.
    pub(crate) fn is_connected(&self, id: usize) -> bool {
        let mut at = id;
        while let Some(p) = self.node(at).parent {
            at = p;
        }
        at == self.root()
    }

    /// The node name for scripts: an HTML element's qualified name in upper
    /// case (DOM §4.9 `tagName`), any other element's as it is.
    pub(crate) fn node_name(&self, id: usize) -> String {
        match &self.node(id).data {
            NodeData::Document => "#document".to_owned(),
            NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                let mut name = qualified_name(&e.name);
                if e.name.ns == ns!(html) {
                    name.make_ascii_uppercase();
                }
                name
            }
            NodeData::Text(_) => "#text".to_owned(),
            NodeData::Comment => "#comment".to_owned(),
        }
    }

    pub(crate) fn element_name(&self, id: usize) -> Option<&QualName> {
        self.node(id).element_data().map(|e| &e.name)
    }

    /// The attribute named `name`, lower-cased first on HTML elements (DOM
    /// "get an attribute by name").
    fn find_attribute(&self, id: usize, name: &str) -> Option<&blitz_dom::Attribute> {
        let e = self.node(id).element_data()?;
        let lower;
        let name = if e.name.ns == ns!(html) {
            lower = name.to_ascii_lowercase();
            lower.as_str()
        } else {
            name
        };
        e.attrs().iter().find(|a| qualified_is(&a.name, name))
    }

    pub(crate) fn attribute(&self, id: usize, name: &str) -> Option<String> {
        self.find_attribute(id, name).map(|a| a.value.clone())
    }

    pub(crate) fn attribute_count(&self, id: usize) -> usize {
        self.node(id).attrs().map_or(0, |a| a.len())
    }

    /// `setAttribute` on an element: `name` already validated.
    pub(crate) fn set_attribute(
        &mut self,
        id: usize,
        name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        let qual = match self.find_attribute(id, name) {
            Some(a) => a.name.clone(),
            None => {
                let html = self.element_name(id).is_some_and(|n| n.ns == ns!(html));
                let local = if html {
                    name.to_ascii_lowercase()
                } else {
                    name.to_owned()
                };
                QualName::new(None, ns!(), LocalName::from(local))
            }
        };
        self.write_attribute(id, qual, value)
    }

    /// Writes one attribute through blitz-dom, charging its bytes.
    pub(crate) fn write_attribute(
        &mut self,
        id: usize,
        name: QualName,
        value: &str,
    ) -> Result<(), DomError> {
        let credit = self.attribute_bytes(id, &name);
        let charge = attribute_cost(&name, value);
        self.recharge(credit, charge)?;
        self.inner.mutate().set_attribute(id, name, value);
        Ok(())
    }

    /// What attribute `name` of `id` is charged at the moment.
    fn attribute_bytes(&self, id: usize, name: &QualName) -> usize {
        self.node(id)
            .attrs()
            .and_then(|l| l.iter().find(|a| a.name == *name))
            .map_or(0, |a| attribute_cost(&a.name, &a.value))
    }

    /// Bytes `id` itself holds.
    fn node_bytes(&self, id: usize) -> usize {
        let node = self.node(id);
        match &node.data {
            NodeData::Text(_) | NodeData::Comment => self.char_data(id).len(),
            NodeData::Element(e) | NodeData::AnonymousBlock(e) => e
                .attrs()
                .iter()
                .map(|a| attribute_cost(&a.name, &a.value))
                .sum(),
            NodeData::Document => 0,
        }
    }

    /// Bytes the subtree at `top` holds, iteratively.
    fn bytes_under(&self, top: usize) -> usize {
        let mut total = 0usize;
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            total = total.saturating_add(self.node_bytes(n));
            stack.extend(self.node(n).children.iter().copied());
        }
        total
    }

    /// Swaps `credit` held bytes for `charge`, failing with
    /// [`DomError::TextLimit`] (and changing nothing) if that's over the
    /// limit. Callers charge before they write.
    fn recharge(&mut self, credit: usize, charge: usize) -> Result<(), DomError> {
        let held = self
            .held_bytes
            .saturating_sub(credit)
            .saturating_add(charge);
        if charge > credit && held > self.limits.max_text_bytes {
            return Err(DomError::TextLimit {
                limit: self.limits.max_text_bytes,
            });
        }
        self.held_bytes = held;
        Ok(())
    }

    /// `removeAttribute`: false if there was no such attribute.
    pub(crate) fn remove_attribute(&mut self, id: usize, name: &str) -> bool {
        match self.find_attribute(id, name).map(|a| a.name.clone()) {
            Some(q) => {
                self.clear_attribute(id, q);
                true
            }
            None => false,
        }
    }

    pub(crate) fn clear_attribute(&mut self, id: usize, name: QualName) {
        let credit = self.attribute_bytes(id, &name);
        self.held_bytes = self.held_bytes.saturating_sub(credit);
        let is_link_href = self
            .element_name(id)
            .is_some_and(|n| n.local.as_ref() == "link")
            && name.local.as_ref() == "href";
        if is_link_href {
            // blitz-dom's clear_attribute unloads the link's stylesheet here
            // and hits `unreachable!` if it has none, which it never does:
            // nothing loads. Nothing else depends on this attribute.
            if let Some(e) = self
                .inner
                .get_node_mut(id)
                .and_then(|n| n.element_data_mut())
            {
                e.attrs.remove(&name);
            }
        } else {
            self.inner.mutate().clear_attribute(id, name);
        }
    }

    /// A text node's or comment's data.
    pub(crate) fn char_data(&self, id: usize) -> &str {
        match (self.side_data.get(&id), &self.node(id).data) {
            (Some(s), _) => s,
            (None, NodeData::Text(t)) => &t.content,
            _ => "",
        }
    }

    /// Writes a comment's data, or a text node's that blitz-dom must not
    /// see, to the side table.
    fn set_side_data(&mut self, id: usize, data: &str) -> Result<(), DomError> {
        self.recharge(self.char_data(id).len(), data.len())?;
        self.side_data.insert(id, data.to_owned());
        Ok(())
    }

    /// Whether `id` is under a `<style>` or is one, by blitz-dom's test: the
    /// local name alone.
    fn is_in_style(&self, id: usize) -> bool {
        let mut at = Some(id);
        while let Some(n) = at {
            if self
                .element_name(n)
                .is_some_and(|q| q.local.as_ref() == "style")
            {
                return true;
            }
            at = self.parent(n);
        }
        false
    }

    /// Moves the data of text nodes under `top` to the side table.
    fn stash_text(&mut self, top: usize) {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            let Some(node) = self.inner.get_node_mut(n) else {
                continue;
            };
            stack.extend(node.children.iter().copied());
            if let Some(t) = node.text_data_mut() {
                let text = std::mem::take(&mut t.content);
                self.side_data.entry(n).or_insert(text);
            }
        }
    }

    /// `textContent`: `None` for the document (the DOM's null).
    pub(crate) fn text_content(&self, id: usize) -> Option<String> {
        match &self.node(id).data {
            NodeData::Document => None,
            NodeData::Text(_) | NodeData::Comment => Some(self.char_data(id).to_owned()),
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                // Iterative, so a deep tree can't overflow the stack.
                let mut out = String::new();
                let mut stack: Vec<usize> = self.node(id).children.iter().rev().copied().collect();
                while let Some(n) = stack.pop() {
                    let node = self.node(n);
                    match &node.data {
                        NodeData::Text(_) => out.push_str(self.char_data(n)),
                        NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                            stack.extend(node.children.iter().rev().copied());
                        }
                        _ => {}
                    }
                }
                Some(out)
            }
        }
    }

    /// The `textContent` setter: elements lose their children (dropped, and
    /// their handles retired) and get one text node; text nodes change
    /// their data; the document ignores it.
    pub(crate) fn set_text_content(&mut self, id: usize, text: &str) -> Result<(), DomError> {
        match self.node(id).data {
            NodeData::Document => Ok(()),
            NodeData::Text(_) => self.set_text_data(id, text),
            NodeData::Comment => self.set_side_data(id, text),
            NodeData::AnonymousBlock(_) => Ok(()),
            NodeData::Element(_) => {
                // Charged first: a refused write leaves the children alone.
                let t = if text.is_empty() {
                    None
                } else {
                    Some(self.create_text(text)?)
                };
                self.drop_children(id);
                if let Some(t) = t {
                    self.join(id, t);
                }
                Ok(())
            }
        }
    }

    pub(crate) fn set_text_data(&mut self, id: usize, text: &str) -> Result<(), DomError> {
        if self.side_data.contains_key(&id) || self.is_in_style(id) {
            return self.set_side_data(id, text);
        }
        self.recharge(self.char_data(id).len(), text.len())?;
        self.inner.mutate().set_node_text(id, text);
        Ok(())
    }

    /// Drops every descendant of `id`, retiring their handles.
    ///
    /// Iterative: blitz-dom's own `remove_and_drop_all_children` recurses
    /// once per level. Every grandchild and below is cut loose (parent and
    /// children links cleared, so each is a leaf with no parent) and
    /// dropped on its own; then blitz drops `id`'s children, which are
    /// leaves by then, and does its restyle bookkeeping for `id`. Each node
    /// is visited once, so a wide tree costs no more than a deep one.
    fn drop_children(&mut self, id: usize) {
        let children = self.node(id).children.clone();
        let mut stack = Vec::new();
        for &c in &children {
            self.retire(c);
            stack.extend(self.take_children(c));
        }
        while let Some(n) = stack.pop() {
            self.retire(n);
            stack.extend(self.take_children(n));
            if let Some(node) = self.inner.get_node_mut(n) {
                node.parent = None;
            }
            // A leaf with no parent: blitz-dom neither recurses nor touches
            // another node.
            self.inner.mutate().remove_and_drop_node(n);
        }
        self.inner.mutate().remove_and_drop_all_children(id);
    }

    fn take_children(&mut self, id: usize) -> Vec<usize> {
        self.inner
            .get_node_mut(id)
            .map(|n| std::mem::take(&mut n.children))
            .unwrap_or_default()
    }

    /// Retires node `id`'s handle, if it has one.
    fn retire(&mut self, id: usize) {
        self.held_bytes = self.held_bytes.saturating_sub(self.node_bytes(id));
        self.side_data.remove(&id);
        if let Some(h) = self.node_to_handle.get_mut(id) {
            if *h != 0 {
                self.handle_to_node[*h as usize - 1] = None;
                *h = 0;
            }
        }
    }

    fn check_room(&self) -> Result<(), DomError> {
        if self.node_count() >= self.limits.max_nodes {
            return Err(DomError::NodeLimit {
                limit: self.limits.max_nodes,
            });
        }
        Ok(())
    }

    pub(crate) fn create_text(&mut self, text: &str) -> Result<usize, DomError> {
        self.check_room()?;
        self.recharge(0, text.len())?;
        Ok(self.inner.mutate().create_text_node(text))
    }

    pub(crate) fn create_comment(&mut self, data: &str) -> Result<usize, DomError> {
        self.check_room()?;
        self.recharge(0, data.len())?;
        let id = self.inner.mutate().create_comment_node();
        self.side_data.insert(id, data.to_owned());
        Ok(id)
    }

    /// `createElement` for an HTML element; `None` for a name the DOM
    /// rejects with InvalidCharacterError.
    pub(crate) fn create_element(&mut self, name: &str) -> Result<Option<usize>, DomError> {
        if !is_valid_element_name(name) {
            return Ok(None);
        }
        self.check_room()?;
        let qual = QualName::new(None, ns!(html), LocalName::from(name.to_ascii_lowercase()));
        Ok(Some(self.inner.mutate().create_element(qual, Vec::new())))
    }

    /// Depth of `id` below the document node, and whether `ancestor` is an
    /// inclusive ancestor of it.
    fn depth_and_ancestry(&self, id: usize, ancestor: usize) -> (usize, bool) {
        let mut depth = 0;
        let mut found = id == ancestor;
        let mut at = id;
        while let Some(p) = self.node(at).parent {
            depth += 1;
            found |= p == ancestor;
            at = p;
        }
        (depth, found)
    }

    /// Height of the subtree at `id` (1 for a leaf), iteratively.
    fn height(&self, id: usize) -> usize {
        let mut best = 0;
        let mut stack = vec![(id, 1usize)];
        while let Some((n, h)) = stack.pop() {
            best = best.max(h);
            stack.extend(self.node(n).children.iter().map(|&c| (c, h + 1)));
        }
        best
    }

    /// `appendChild`. `Ok(false)` for the DOM's HierarchyRequestError.
    pub(crate) fn append_child(&mut self, parent: usize, child: usize) -> Result<bool, DomError> {
        // §4.2.3: a document takes comments here; `documentElement` already
        // exists after a parse, so elements are refused either way.
        let parent_ok = match self.kind(parent) {
            Some(Kind::Element) => true,
            Some(Kind::Document) => self.kind(child) == Some(Kind::Comment),
            _ => false,
        };
        let child_ok = matches!(
            self.node(child).data,
            NodeData::Element(_) | NodeData::Text(_) | NodeData::Comment
        );
        let (depth, child_is_ancestor) = self.depth_and_ancestry(parent, child);
        if !parent_ok || !child_ok || child_is_ancestor {
            return Ok(false);
        }
        if depth + self.height(child) > self.limits.max_depth {
            return Err(DomError::TreeTooDeep {
                limit: self.limits.max_depth,
            });
        }
        self.join(parent, child);
        Ok(true)
    }

    /// Appends `child` to `parent`. `child` leaves any old parent first:
    /// blitz-dom's own move removes the node from the old parent's list by
    /// id after adding it to the new one, so moving within one parent would
    /// lose the node and leave a parent link nothing points back to.
    fn join(&mut self, parent: usize, child: usize) {
        if self.node(child).parent.is_some() {
            self.inner.mutate().remove_node(child);
        }
        if self.is_in_style(parent) {
            self.stash_text(child);
        }
        self.inner.mutate().append_children(parent, &[child]);
    }

    /// Checks the tree's links and the byte count; for tests. Returns what
    /// is wrong, if anything.
    #[doc(hidden)]
    pub fn check_consistency(&self) -> Result<(), String> {
        let mut held = 0usize;
        for (id, node) in self.inner.tree().iter() {
            held = held.saturating_add(self.node_bytes(id));
            if let Some(p) = node.parent {
                let Some(parent) = self.inner.get_node(p) else {
                    return Err(format!("node {id} has a dropped parent {p}"));
                };
                if parent.children.iter().filter(|&&c| c == id).count() != 1 {
                    return Err(format!("node {id} isn't once in its parent {p}'s children"));
                }
            }
            for &c in &node.children {
                if self.inner.get_node(c).and_then(|n| n.parent) != Some(id) {
                    return Err(format!("child {c} of {id} doesn't point back"));
                }
            }
        }
        if held != self.held_bytes {
            return Err(format!("{held} bytes held, {} counted", self.held_bytes));
        }
        Ok(())
    }
}

/// What one attribute is charged.
fn attribute_cost(name: &QualName, value: &str) -> usize {
    ATTRIBUTE_COST + name.local.len() + value.len()
}

/// An element's or attribute's qualified name (`prefix:local`).
pub(crate) fn qualified_name(name: &QualName) -> String {
    match &name.prefix {
        Some(p) => format!("{p}:{}", name.local),
        None => name.local.to_string(),
    }
}

/// Whether `name`'s qualified name is `wanted`, without building it.
fn qualified_is(name: &QualName, wanted: &str) -> bool {
    match &name.prefix {
        Some(p) => wanted
            .strip_prefix(p.as_ref())
            .and_then(|r| r.strip_prefix(':'))
            .is_some_and(|r| r == name.local.as_ref()),
        None => wanted == name.local.as_ref(),
    }
}

/// DOM "valid element local name" (§1.5, namespaces).
pub(crate) fn is_valid_element_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        !name.contains(['\t', '\n', '\u{c}', '\r', ' ', '\0', '/', '>'])
    } else {
        (first == ':' || first == '_' || first >= '\u{80}')
            && chars.all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_') || c >= '\u{80}'
            })
    }
}

/// DOM "valid attribute local name" (§1.5, namespaces).
pub(crate) fn is_valid_attribute_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\t', '\n', '\u{c}', '\r', ' ', '\0', '/', '=', '>'])
}

/// Writes an attribute's serialized name (HTML Standard §13.3, "serializing
/// HTML fragments"): the local name without a namespace; `xml:`, `xmlns:`
/// or `xlink:` plus the local name in those namespaces (`xmlns` alone for
/// the `xmlns` attribute itself); the qualified name otherwise.
fn push_attribute_name(name: &QualName, out: &mut String) {
    let prefix = if name.ns == ns!() {
        None
    } else if name.ns == ns!(xml) {
        Some("xml")
    } else if name.ns == ns!(xmlns) {
        (name.local != *"xmlns").then_some("xmlns")
    } else if name.ns == ns!(xlink) {
        Some("xlink")
    } else {
        name.prefix.as_ref().map(|p| p.as_ref())
    };
    if let Some(prefix) = prefix {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(&name.local);
}

/// Escapes text or an attribute value as HTML serialization does (HTML
/// Standard §13.3, "escaping a string").
fn escape_into(s: &str, attribute: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attribute => out.push_str("&quot;"),
            // Escaped in attribute values too since the 2025 change to the
            // algorithm.
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}
