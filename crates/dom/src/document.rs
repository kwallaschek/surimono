//! A parsed document and the handle table that scripts address it through.

use std::sync::atomic::{AtomicU64, Ordering};

use blitz_dom::{BaseDocument, DocumentConfig, LocalName, NodeData, QualName, ns};
use blitz_html::HtmlDocument;

use crate::DomError;

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
}

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

    /// Checks the limits are in range: every limit at least 1, `max_depth`
    /// at most [`MAX_DEPTH`](Self::MAX_DEPTH), `max_handles` within `u32`.
    pub fn validate(&self) -> Result<(), DomError> {
        let bad = |what: &str| {
            Err(DomError::InvalidLimits {
                message: what.to_owned(),
            })
        };
        if self.max_nodes == 0 || self.max_depth == 0 || self.max_handles == 0 {
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
        let inner = HtmlDocument::from_html(html, DocumentConfig::default()).into_inner();
        let doc = Self {
            inner,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            limits,
            handle_to_node: Vec::new(),
            node_to_handle: Vec::new(),
        };
        if doc.node_count() > limits.max_nodes {
            return Err(DomError::NodeLimit {
                limit: limits.max_nodes,
            });
        }
        if doc.height(doc.root()) - 1 > limits.max_depth {
            return Err(DomError::TreeTooDeep {
                limit: limits.max_depth,
            });
        }
        Ok(doc)
    }

    /// The document's serialization: the `<html>` element's outer HTML, by
    /// the HTML Standard's fragment serialization algorithm (§13.3) for
    /// HTML elements, text and comments. Iterative, so tree depth can't
    /// overflow the stack.
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
                        NodeData::Text(t) => {
                            let raw = node
                                .parent
                                .and_then(|p| self.node(p).element_data())
                                .is_some_and(|e| {
                                    matches!(
                                        e.name.local.as_ref(),
                                        "style"
                                            | "script"
                                            | "xmp"
                                            | "iframe"
                                            | "noembed"
                                            | "noframes"
                                            | "plaintext"
                                            | "noscript"
                                    )
                                });
                            if raw {
                                out.push_str(&t.content);
                            } else {
                                escape_into(&t.content, false, out);
                            }
                        }
                        // blitz-dom 0.2.4 keeps no comment data.
                        NodeData::Comment => out.push_str("<!---->"),
                        NodeData::Document => {}
                        NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                            out.push('<');
                            out.push_str(&e.name.local);
                            for a in e.attrs() {
                                out.push(' ');
                                out.push_str(&a.name.local);
                                out.push_str("=\"");
                                escape_into(&a.value, true, out);
                                out.push('"');
                            }
                            out.push('>');
                            let void = matches!(
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

    fn node(&self, id: usize) -> &blitz_dom::Node {
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

    pub(crate) fn body(&self) -> Option<usize> {
        let html = self.document_element()?;
        self.node(html).children.iter().copied().find(|&c| {
            self.node(c)
                .element_data()
                .is_some_and(|e| e.name.local.as_ref() == "body")
        })
    }

    pub(crate) fn element_by_id(&self, id: &str) -> Option<usize> {
        self.inner.get_element_by_id(id)
    }

    pub(crate) fn parent(&self, id: usize) -> Option<usize> {
        self.node(id).parent
    }

    pub(crate) fn node_name(&self, id: usize) -> String {
        match &self.node(id).data {
            NodeData::Document => "#document".to_owned(),
            NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                e.name.local.as_ref().to_ascii_uppercase()
            }
            NodeData::Text(_) => "#text".to_owned(),
            NodeData::Comment => "#comment".to_owned(),
        }
    }

    pub(crate) fn attribute(&self, id: usize, name: &str) -> Option<String> {
        let name = name.to_ascii_lowercase();
        self.node(id)
            .attrs()?
            .iter()
            .find(|a| a.name.ns == ns!() && a.name.local.as_ref() == name)
            .map(|a| a.value.clone())
    }

    /// `textContent`: `None` for the document (the DOM's null).
    pub(crate) fn text_content(&self, id: usize) -> Option<String> {
        match &self.node(id).data {
            NodeData::Document => None,
            NodeData::Text(t) => Some(t.content.clone()),
            NodeData::Comment => Some(String::new()),
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                // Iterative, so a deep tree can't overflow the stack.
                let mut out = String::new();
                let mut stack: Vec<usize> = self.node(id).children.iter().rev().copied().collect();
                while let Some(n) = stack.pop() {
                    let node = self.node(n);
                    match &node.data {
                        NodeData::Text(t) => out.push_str(&t.content),
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
            NodeData::Text(_) => {
                self.inner.mutate().set_node_text(id, text);
                Ok(())
            }
            NodeData::Comment | NodeData::AnonymousBlock(_) => Ok(()),
            NodeData::Element(_) => {
                self.drop_children(id);
                if !text.is_empty() {
                    let t = self.create_text(text)?;
                    self.inner.mutate().append_children(id, &[t]);
                }
                Ok(())
            }
        }
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
        Ok(self.inner.mutate().create_text_node(text))
    }

    /// `createElement` for an HTML element; `None` for a name the DOM
    /// rejects with InvalidCharacterError.
    pub(crate) fn create_element(&mut self, name: &str) -> Result<Option<usize>, DomError> {
        let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        if !valid {
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
        let parent_ok = matches!(self.node(parent).data, NodeData::Element(_));
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
        self.inner.mutate().append_children(parent, &[child]);
        Ok(true)
    }
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
