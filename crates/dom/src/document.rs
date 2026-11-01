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
    pub max_nodes: usize,
    /// Deepest nesting an insertion may create, counted from the document
    /// node. Bounds the recursion in blitz-dom's subtree walks.
    pub max_depth: usize,
}

impl DomLimits {
    /// 100,000 nodes: a large report page has a few thousand.
    pub const DEFAULT_MAX_NODES: usize = 100_000;
    /// 512 levels, as Blink's HTML parser caps nesting.
    pub const DEFAULT_MAX_DEPTH: usize = 512;
}

impl Default for DomLimits {
    fn default() -> Self {
        Self {
            max_nodes: Self::DEFAULT_MAX_NODES,
            max_depth: Self::DEFAULT_MAX_DEPTH,
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
    pub fn parse(html: &str) -> Self {
        Self::parse_with_limits(html, DomLimits::default())
    }

    /// Parses `html`. No network, no system fonts: blitz-dom is built
    /// without them and the config has no providers.
    pub fn parse_with_limits(html: &str, limits: DomLimits) -> Self {
        let inner = HtmlDocument::from_html(html, DocumentConfig::default()).into_inner();
        Self {
            inner,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            limits,
            handle_to_node: Vec::new(),
            node_to_handle: Vec::new(),
        }
    }

    /// The document's serialization (the `<html>` element's outer HTML).
    pub fn to_html(&self) -> String {
        match self.document_element() {
            Some(id) => self.node(id).outer_html(),
            None => String::new(),
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

    /// The handle for blitz node `id`, made on first use.
    pub(crate) fn handle_for(&mut self, id: usize) -> Handle {
        if self.node_to_handle.len() <= id {
            self.node_to_handle.resize(id + 1, 0);
        }
        if self.node_to_handle[id] == 0 {
            self.handle_to_node.push(Some(id));
            // At most `max_nodes` handles live at once, but dropped nodes'
            // handles stay retired, so a script that churns nodes could in
            // theory count past u32; saturate rather than wrap.
            self.node_to_handle[id] = Handle::try_from(self.handle_to_node.len()).unwrap_or(0);
        }
        self.node_to_handle[id]
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
            // dropped node's or handle 0 from saturation.
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

    /// Drops every descendant of `id`, retiring their handles first.
    fn drop_children(&mut self, id: usize) {
        let mut stack: Vec<usize> = self.node(id).children.clone();
        while let Some(n) = stack.pop() {
            if let Some(h) = self.node_to_handle.get_mut(n) {
                if *h != 0 {
                    self.handle_to_node[*h as usize - 1] = None;
                    *h = 0;
                }
            }
            stack.extend(self.node(n).children.iter().copied());
        }
        self.inner.mutate().remove_and_drop_all_children(id);
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
