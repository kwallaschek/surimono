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
    DocumentFragment = 11,
}

/// The element name that marks a `DocumentFragment`: blitz-dom has no such
/// node, and a name no parser or `createElement` can produce can't clash.
/// A fragment is never inserted (its children are), so nothing but this
/// crate sees it.
const FRAGMENT_NAME: &str = "#document-fragment";

/// The local name marking a fragment.
pub(crate) fn fragment_name() -> LocalName {
    LocalName::from(FRAGMENT_NAME)
}

/// Whether `name` is the marker of a fragment.
pub(crate) fn is_fragment_name(name: &QualName) -> bool {
    name.local.as_ref() == FRAGMENT_NAME
}

/// Which elements a live list holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ListKind {
    /// `childNodes`.
    ChildNodes,
    /// `children`.
    Children,
    /// `getElementsByTagName`.
    ByTag(String),
    /// `getElementsByClassName`.
    ByClass(String),
}

/// One script-visible object behind a handle. Everything but `Node` is
/// addressed by its owner node, which has a handle too and so stays alive.
#[derive(Debug, Clone)]
pub(crate) enum Object {
    Node(usize),
    /// A live `NodeList` or `HTMLCollection` under a node.
    Live(usize, ListKind),
    /// The static `NodeList` from `querySelectorAll`.
    Static(Vec<usize>),
    /// `classList`.
    Tokens(usize),
    /// `attributes`.
    Attributes(usize),
    /// `dataset`.
    Dataset(usize),
    /// `style`.
    Style(usize),
    /// A `MutationRecord`.
    Record(Box<MutationRecord>),
    /// An `Attr`: its element, its name, and its value if removed since.
    Attr(usize, QualName, Option<String>),
}

/// A `MutationRecord`'s fields (DOM §4.3.3). Nodes have handles.
#[derive(Debug, Clone)]
pub(crate) struct MutationRecord {
    pub kind: &'static str,
    pub target: usize,
    pub added: Handle,
    pub removed: Handle,
    pub previous: Option<usize>,
    pub next: Option<usize>,
    pub attribute: Option<String>,
    pub namespace: Option<String>,
    pub old_value: Option<String>,
}

/// One `observe` on one node (DOM §4.3.1).
#[derive(Debug, Clone)]
struct Registration {
    observer: u32,
    flags: u32,
    filter: Vec<String>,
}

const CHILD_LIST: u32 = 1;
const ATTRIBUTES: u32 = 2;
const CHARACTER_DATA: u32 = 4;
const SUBTREE: u32 = 8;
const ATTRIBUTE_OLD: u32 = 16;
const CHARACTER_OLD: u32 = 32;
const HAS_FILTER: u32 = 64;

/// A per-owner object: one handle each, for `[SameObject]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Owned {
    List(ListKind),
    Tokens,
    Attributes,
    Dataset,
    Style,
    Attr(QualName),
}

/// A handle as JavaScript holds it, with the document's generation. Neither
/// is a pointer or a blitz node id.
pub(crate) type Handle = u32;

/// A document scripts can run against: a blitz-dom document plus the table
/// that maps the handles given to JavaScript to blitz node ids.
///
/// A node with a handle is never dropped before the document: a script may
/// still hold it, and the DOM keeps removed nodes usable. A removed subtree
/// with no handle in it can't be reached again, so it is dropped at once.
pub struct Document {
    inner: BaseDocument,
    generation: u64,
    limits: DomLimits,
    /// Handle (1-based) minus one to its object.
    objects: Vec<Object>,
    /// The handles of per-owner objects. Only looked up, so its hash order
    /// can't reach any output.
    owned: HashMap<(usize, Owned), Handle>,
    /// Bumped by every change a live list can see.
    version: u64,
    /// The last computed live list: (handle, version, nodes), so a loop
    /// over `list[i]` isn't quadratic. One entry bounds what it holds.
    list_cache: Option<(Handle, u64, Vec<usize>)>,
    /// Entries in static lists so far, bounded by [`Self::static_limit`].
    static_entries: usize,
    /// Mutation observers: registrations by node, and each observer's queue
    /// of record handles, by observer number. Empty until a page observes,
    /// and mutations check that first.
    registrations: HashMap<usize, Vec<Registration>>,
    record_queues: Vec<Vec<Handle>>,
    /// See [`Document::empty_list`]; 0 until made.
    empty_list: Handle,
    /// `<template>` to its content fragment.
    templates: HashMap<usize, usize>,
    /// The running call's time limit, for work inside one native call.
    pub(crate) deadline: Option<surimono_core::Deadline>,
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
    /// `readyState`: 0 loading, 1 interactive, 2 complete. A document no
    /// page lifecycle has run on is complete.
    ready_state: u32,
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
            .field("handles", &self.objects.len())
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
            objects: Vec::new(),
            owned: HashMap::new(),
            version: 0,
            list_cache: None,
            static_entries: 0,
            registrations: HashMap::new(),
            record_queues: Vec::new(),
            empty_list: 0,
            templates: parsed.templates.into_iter().collect(),
            deadline: None,
            node_to_handle: Vec::new(),
            side_data: parsed.strings.into_iter().collect(),
            held_bytes: 0,
            quirks: parsed.quirks,
            ready_state: 2,
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

    /// Parses `html` in the context of element `context` into a new
    /// detached fragment, all of which fits the limits under `parent`.
    /// `Ok(None)` if the time limit passed meanwhile.
    pub(crate) fn parse_into_fragment(
        &mut self,
        context: usize,
        parent: usize,
        html: &str,
    ) -> Result<Option<usize>, DomError> {
        let holder = self.create_fragment()?;
        let room = self.limits.max_nodes.saturating_sub(self.node_count());
        // The parser's own root and holder add two levels.
        let levels = self.limits.max_depth.saturating_sub(self.depth(parent)) + 2;
        let deadline = self.deadline.clone();
        let parsed = parse::parse_fragment(
            &mut self.inner.mutate(),
            context,
            holder,
            html,
            ParseLimits {
                max_nodes: room,
                max_depth: levels,
            },
            || {
                deadline
                    .as_ref()
                    .is_some_and(surimono_core::Deadline::passed)
            },
        );
        self.side_data.extend(parsed.strings);
        // Charged here and credited by the drops below if it is refused.
        self.held_bytes = self.held_bytes.saturating_add(self.bytes_under(holder));
        // Holder → the parser's root → the content, without the root.
        let root = self.take_children(holder);
        let mut content = Vec::new();
        for &r in &root {
            self.set_parent(r, None);
            content.extend(self.take_children(r));
            self.drop_subtree(r);
        }
        for &c in &content {
            self.set_parent(c, Some(holder));
        }
        if let Some(n) = self.inner.get_node_mut(holder) {
            n.children = content;
        }
        let timed_out = deadline
            .as_ref()
            .is_some_and(surimono_core::Deadline::passed);
        let limit = if self.node_count() > self.limits.max_nodes {
            Some(DomError::NodeLimit {
                limit: self.limits.max_nodes,
            })
        } else if self.held_bytes > self.limits.max_text_bytes {
            Some(DomError::TextLimit {
                limit: self.limits.max_text_bytes,
            })
        } else {
            self.check_depth(parent, self.height(holder) - 1).err()
        };
        if limit.is_some() || timed_out || parsed.over_limit {
            self.drop_subtree(holder);
            return match limit {
                Some(e) => Err(e),
                None if timed_out => Ok(None),
                None => Err(DomError::NodeLimit {
                    limit: self.limits.max_nodes,
                }),
            };
        }
        Ok(Some(holder))
    }

    /// `id`'s children, serialized (HTML §13.3, `innerHTML`).
    pub(crate) fn inner_html(&self, id: usize) -> String {
        let mut out = String::new();
        let top = self.templates.get(&id).copied().unwrap_or(id);
        for &c in self.children(top) {
            self.serialize(c, &mut out);
        }
        out
    }

    /// `id` itself, serialized (`outerHTML`).
    pub(crate) fn outer_html(&self, id: usize) -> String {
        let mut out = String::new();
        self.serialize(id, &mut out);
        out
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
                                // §13.3: a template's content, not its
                                // children, of which it has none when parsed.
                                let content = self.templates.get(&id).copied();
                                if let Some(c) = content {
                                    let kids = &self.node(c).children;
                                    stack.extend(kids.iter().rev().map(|&k| Step::Open(k)));
                                }
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
        // `drop_subtree`, on nodes without handles.
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
            self.node_to_handle[id] = self.new_handle(Object::Node(id))?;
        }
        Ok(self.node_to_handle[id])
    }

    fn new_handle(&mut self, object: Object) -> Result<Handle, DomError> {
        if self.objects.len() >= self.limits.max_handles {
            return Err(DomError::HandleLimit {
                limit: self.limits.max_handles,
            });
        }
        self.objects.push(object);
        // `validate` keeps max_handles within u32, so this always fits; 0
        // (never a valid handle) otherwise.
        Ok(Handle::try_from(self.objects.len()).unwrap_or(0))
    }

    /// The handle of `owner`'s object `key`, made on first use.
    pub(crate) fn owned_handle(
        &mut self,
        owner: usize,
        key: Owned,
        make: impl FnOnce() -> Object,
    ) -> Result<Handle, DomError> {
        if let Some(&h) = self.owned.get(&(owner, key.clone())) {
            return Ok(h);
        }
        let h = self.new_handle(make())?;
        self.owned.insert((owner, key), h);
        Ok(h)
    }

    /// A new static list of `nodes`, which get handles so they stay alive.
    /// Every entry counts: lists live in Rust, outside the JS heap limit.
    pub(crate) fn static_list(&mut self, nodes: Vec<usize>) -> Result<Handle, DomError> {
        self.static_entries = self.static_entries.saturating_add(nodes.len());
        if self.static_entries > self.limits.max_handles.saturating_mul(8) {
            return Err(DomError::HandleLimit {
                limit: self.limits.max_handles,
            });
        }
        for &n in &nodes {
            self.handle_for(n)?;
        }
        self.new_handle(Object::Static(nodes))
    }

    /// Checks `(generation, handle)` names an object.
    pub(crate) fn check(&self, generation: u64, handle: Handle) -> Result<(), DomError> {
        self.resolve(generation, handle).map(|_| ())
    }

    /// The object that `(generation, handle)` names.
    pub(crate) fn resolve(&self, generation: u64, handle: Handle) -> Result<&Object, DomError> {
        if generation != self.generation {
            return Err(DomError::StaleDocument);
        }
        handle
            .checked_sub(1)
            .and_then(|i| self.objects.get(i as usize))
            // Handles only come from `new_handle`, so an unknown one can't
            // be this document's.
            .ok_or(DomError::StaleDocument)
    }

    pub(crate) fn object_mut(&mut self, handle: Handle) -> Option<&mut Object> {
        self.objects.get_mut(handle.checked_sub(1)? as usize)
    }

    /// The nodes of live list `kind` under `owner`, cached for `handle`.
    pub(crate) fn live_nodes(&mut self, handle: Handle, owner: usize, kind: &ListKind) -> &[usize] {
        let fresh = self
            .list_cache
            .as_ref()
            .is_some_and(|(h, v, _)| *h == handle && *v == self.version);
        if !fresh {
            let nodes = self.compute_list(owner, kind);
            self.list_cache = Some((handle, self.version, nodes));
        }
        self.list_cache
            .as_ref()
            .map_or(&[], |(_, _, n)| n.as_slice())
    }

    /// Descendant elements of `top` in tree order that satisfy `keep`.
    pub(crate) fn descendants(
        &self,
        top: usize,
        mut keep: impl FnMut(usize) -> bool,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.children(top).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if self.kind(n) == Some(Kind::Element) && keep(n) {
                out.push(n);
            }
            stack.extend(self.children(n).iter().rev().copied());
        }
        out
    }

    fn compute_list(&self, owner: usize, kind: &ListKind) -> Vec<usize> {
        match kind {
            ListKind::ChildNodes => self.children(owner).to_vec(),
            ListKind::Children => self
                .children(owner)
                .iter()
                .copied()
                .filter(|&c| self.kind(c) == Some(Kind::Element))
                .collect(),
            ListKind::ByTag(name) => {
                let lower = name.to_ascii_lowercase();
                self.descendants(owner, |n| {
                    let Some(e) = self.element_name(n) else {
                        return false;
                    };
                    // DOM §4.2.6.2: lower-cased against HTML elements.
                    let wanted = if e.ns == ns!(html) { &lower } else { name };
                    name == "*" || qualified_is(e, wanted)
                })
            }
            ListKind::ByClass(names) => {
                let wanted: Vec<&str> = names.split_ascii_whitespace().collect();
                if wanted.is_empty() {
                    return Vec::new();
                }
                self.descendants(owner, |n| {
                    let class = self.attribute(n, "class").unwrap_or_default();
                    wanted.iter().all(|w| {
                        class.split_ascii_whitespace().any(|c| {
                            // Case-insensitive in quirks mode (DOM §4.2.6.2).
                            c == *w || (self.quirks && c.eq_ignore_ascii_case(w))
                        })
                    })
                })
            }
        }
    }

    pub(crate) fn kind(&self, id: usize) -> Option<Kind> {
        match &self.node(id).data {
            NodeData::Document => Some(Kind::Document),
            NodeData::Element(e) if e.name.local.as_ref() == FRAGMENT_NAME => {
                Some(Kind::DocumentFragment)
            }
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

    /// The first element in tree order under `top` with this ID. blitz-dom's
    /// own map keeps one node per ID and misses IDs set after insertion, so
    /// this walks the tree.
    pub(crate) fn element_by_id(&self, top: usize, id: &str) -> Option<usize> {
        if id.is_empty() {
            return None;
        }
        let mut stack = vec![top];
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

    pub(crate) fn ready_state(&self) -> u32 {
        self.ready_state
    }

    pub(crate) fn set_ready(&mut self, state: u32) {
        self.ready_state = state;
    }

    /// Classic scripts in tree order, for the page lifecycle: (node, `src`
    /// value, text, defer). `module` and other types aren't run (HTML
    /// §4.12.1.1); a parser-inserted `async` script keeps its tree position,
    /// which is one of the orders the spec allows and the deterministic one.
    pub(crate) fn scripts(&self) -> Vec<(usize, Option<String>, String, bool)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            stack.extend(self.children(n).iter().rev().copied());
            let is_script = self
                .element_name(n)
                .is_some_and(|q| q.ns == ns!(html) && q.local.as_ref() == "script");
            if !is_script || !self.is_classic_script(n) {
                continue;
            }
            let src = self.attribute(n, "src");
            let defer = src.is_some() && self.attribute(n, "defer").is_some();
            out.push((n, src, self.text_content(n).unwrap_or_default(), defer));
        }
        out
    }

    /// HTML §4.12.1.1: no `type` or a JavaScript MIME type, or `language`
    /// in the old way. `nomodule` is ignored: modules don't run here, so the
    /// fallback is what a page has for us.
    fn is_classic_script(&self, id: usize) -> bool {
        let ty = self.attribute(id, "type");
        match ty.as_deref().map(str::trim) {
            None | Some("") => true,
            Some(t) => {
                let t = t.to_ascii_lowercase();
                matches!(
                    t.as_str(),
                    "text/javascript"
                        | "application/javascript"
                        | "application/x-javascript"
                        | "text/ecmascript"
                        | "application/ecmascript"
                        | "text/jscript"
                        | "text/x-javascript"
                        | "text/x-ecmascript"
                )
            }
        }
    }

    /// The `<title>` element: the first in tree order (HTML §3.1.3).
    fn title_element(&self) -> Option<usize> {
        let top = self.document_element()?;
        let is_title = |n: usize| {
            self.element_name(n)
                .is_some_and(|q| q.ns == ns!(html) && q.local.as_ref() == "title")
        };
        if is_title(top) {
            return Some(top);
        }
        self.descendants(top, is_title).first().copied()
    }

    /// `document.title`: the title's text, with whitespace stripped and
    /// collapsed.
    pub(crate) fn title(&self) -> String {
        self.title_element()
            .and_then(|t| self.text_content(t))
            .map(|t| t.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }

    /// `document.title` set: the title's content, a new `<title>` in the
    /// head if there is none; nothing without a head.
    pub(crate) fn set_title(&mut self, value: &str) -> Result<(), DomError> {
        let title = match self.title_element() {
            Some(t) => t,
            None => {
                let Some(head) = self.head() else {
                    return Ok(());
                };
                let t = self.create_element("title")?.ok_or(DomError::NoDocument)?;
                self.place(head, &[t], None);
                t
            }
        };
        self.set_text_content(title, value)
    }

    pub(crate) fn is_quirks(&self) -> bool {
        self.quirks
    }

    pub(crate) fn quirks_mode(&self) -> style::context::QuirksMode {
        if self.quirks {
            style::context::QuirksMode::Quirks
        } else {
            style::context::QuirksMode::NoQuirks
        }
    }

    /// `selectors` parsed; `None` if it isn't valid.
    pub(crate) fn selector_list(
        &self,
        selectors: &str,
    ) -> Option<selectors::SelectorList<style::selector_parser::SelectorImpl>> {
        self.inner.try_parse_selector_list(selectors).ok()
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

    /// The attribute in namespace `ns_uri` (empty for none) named `local`.
    pub(crate) fn attribute_ns(&self, id: usize, ns_uri: &str, local: &str) -> Option<QualName> {
        let e = self.node(id).element_data()?;
        e.attrs()
            .iter()
            .find(|a| a.name.ns.as_ref() == ns_uri && a.name.local.as_ref() == local)
            .map(|a| a.name.clone())
    }

    pub(crate) fn attribute_value(&self, id: usize, name: &QualName) -> Option<String> {
        let e = self.node(id).element_data()?;
        e.attrs()
            .iter()
            .find(|a| a.name == *name)
            .map(|a| a.value.clone())
    }

    /// `setAttributeNS`: the attribute named `name`, replacing one with its
    /// namespace and local name.
    pub(crate) fn set_attribute_ns(
        &mut self,
        id: usize,
        name: QualName,
        value: &str,
    ) -> Result<(), DomError> {
        let name = self
            .attribute_ns(id, name.ns.as_ref(), name.local.as_ref())
            .unwrap_or(name);
        self.write_attribute(id, name, value)
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
        self.note_attribute(id, &name);
        self.version += 1;
        self.inner.mutate().set_attribute(id, name, value);
        Ok(())
    }

    /// An `Attr` object for an attribute about to go keeps its value.
    fn keep_attr_value(&mut self, id: usize, name: &QualName) {
        let Some(&h) = self.owned.get(&(id, Owned::Attr(name.clone()))) else {
            return;
        };
        let value = self.attribute_value(id, name);
        if let Some(Object::Attr(_, _, kept)) = self.object_mut(h) {
            *kept = value;
        }
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
            self.push_owned(n, &mut stack);
        }
        total
    }

    /// Pushes what `n` owns: its children, and a template's content, which
    /// is in no tree of its own account.
    fn push_owned(&self, n: usize, stack: &mut Vec<usize>) {
        stack.extend(self.node(n).children.iter().copied());
        if let Some(&c) = self.templates.get(&n) {
            stack.push(c);
        }
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

    /// The qualified names of `id`'s attributes, in order.
    pub(crate) fn attribute_names(&self, id: usize) -> Vec<String> {
        self.node(id)
            .attrs()
            .map(|a| a.iter().map(|a| qualified_name(&a.name)).collect())
            .unwrap_or_default()
    }

    pub(crate) fn attribute_name_at(&self, id: usize, i: usize) -> Option<QualName> {
        self.node(id).attrs()?.get(i).map(|a| a.name.clone())
    }

    pub(crate) fn attribute_named(&self, id: usize, name: &str) -> Option<QualName> {
        self.find_attribute(id, name).map(|a| a.name.clone())
    }

    pub(crate) fn clear_attribute(&mut self, id: usize, name: QualName) {
        self.keep_attr_value(id, &name);
        self.note_attribute(id, &name);
        self.version += 1;
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
        self.note_character_data(id);
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

    /// The `textContent` setter (DOM §4.4): elements and fragments replace
    /// their children with one text node; text nodes and comments change
    /// their data; the document ignores it.
    pub(crate) fn set_text_content(&mut self, id: usize, text: &str) -> Result<(), DomError> {
        match self.kind(id) {
            Some(Kind::Text) => self.set_text_data(id, text),
            Some(Kind::Comment) => self.set_side_data(id, text),
            Some(Kind::Element | Kind::DocumentFragment) => {
                // Made first: a refused write leaves the children alone.
                let t = if text.is_empty() {
                    None
                } else {
                    Some(self.create_text(text)?)
                };
                self.remove_all_children(id);
                if let Some(t) = t {
                    self.place(id, &[t], None);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn set_text_data(&mut self, id: usize, text: &str) -> Result<(), DomError> {
        if self.side_data.contains_key(&id) || self.is_in_style(id) {
            return self.set_side_data(id, text);
        }
        self.recharge(self.char_data(id).len(), text.len())?;
        self.note_character_data(id);
        self.inner.mutate().set_node_text(id, text);
        Ok(())
    }

    /// Removes every child of `id` (DOM "replace all" with nothing). A child
    /// whose subtree holds a handle stays alive, detached; any other is
    /// dropped.
    ///
    /// Iterative, and linear in the subtree: blitz-dom's own subtree drop
    /// recurses per level, and removing children one at a time scans the
    /// parent's list each time.
    pub(crate) fn remove_all_children(&mut self, id: usize) {
        if self.is_observed() && !self.children(id).is_empty() {
            let removed = self.children(id).to_vec();
            self.note_child_list(id, &[], &removed, None, None);
        }
        self.version += 1;
        for c in self.take_children(id) {
            self.set_parent(c, None);
            if self.subtree_has_handle(c) {
                // No parent to update: this only updates blitz-dom's
                // document state for the subtree.
                self.inner.mutate().remove_node(c);
            } else {
                self.drop_subtree(c);
            }
        }
        // `id` has no children left; this does blitz-dom's restyle
        // bookkeeping for it.
        self.inner.mutate().remove_and_drop_all_children(id);
    }

    fn subtree_has_handle(&self, top: usize) -> bool {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            if self.has_handle(n) {
                return true;
            }
            self.push_owned(n, &mut stack);
        }
        false
    }

    /// Drops `top`, which has no parent, and its subtree, except any part
    /// under a node with a handle, which stays alive and detached: a script
    /// may hold it. Every dropped node is cut loose first, so blitz-dom
    /// neither recurses nor touches another node.
    pub(crate) fn drop_subtree(&mut self, top: usize) {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            self.set_parent(n, None);
            if self.has_handle(n) {
                continue;
            }
            stack.extend(self.take_children(n));
            stack.extend(self.templates.remove(&n));
            self.held_bytes = self.held_bytes.saturating_sub(self.node_bytes(n));
            self.side_data.remove(&n);
            self.inner.mutate().remove_and_drop_node(n);
        }
    }

    fn has_handle(&self, id: usize) -> bool {
        self.node_to_handle.get(id).is_some_and(|&h| h != 0)
    }

    pub(crate) fn set_parent(&mut self, id: usize, parent: Option<usize>) {
        if let Some(node) = self.inner.get_node_mut(id) {
            node.parent = parent;
        }
    }

    pub(crate) fn take_children(&mut self, id: usize) -> Vec<usize> {
        self.inner
            .get_node_mut(id)
            .map(|n| std::mem::take(&mut n.children))
            .unwrap_or_default()
    }

    fn check_room(&self) -> Result<(), DomError> {
        self.check_room_for(1)
    }

    /// Fails unless `n` more nodes fit.
    pub(crate) fn check_room_for(&self, n: usize) -> Result<(), DomError> {
        if self.node_count().saturating_add(n) > self.limits.max_nodes {
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

    pub(crate) fn create_fragment(&mut self) -> Result<usize, DomError> {
        self.check_room()?;
        let name = QualName::new(None, ns!(), fragment_name());
        Ok(self.inner.mutate().create_element(name, Vec::new()))
    }

    /// A new element with `name` and copies of `attrs`.
    pub(crate) fn create_element_with(
        &mut self,
        name: QualName,
        attrs: Vec<blitz_dom::Attribute>,
    ) -> Result<usize, DomError> {
        self.check_room()?;
        let bytes = attrs
            .iter()
            .map(|a| attribute_cost(&a.name, &a.value))
            .sum();
        self.recharge(0, bytes)?;
        Ok(self.inner.mutate().create_element(name, attrs))
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

    /// Depth of `id` below the root of its tree.
    pub(crate) fn depth(&self, id: usize) -> usize {
        let mut depth = 0;
        let mut at = id;
        while let Some(p) = self.node(at).parent {
            depth += 1;
            at = p;
        }
        depth
    }

    /// Whether `ancestor` is `id` or an ancestor of it.
    pub(crate) fn is_inclusive_ancestor(&self, ancestor: usize, id: usize) -> bool {
        let mut at = Some(id);
        while let Some(n) = at {
            if n == ancestor {
                return true;
            }
            at = self.node(n).parent;
        }
        false
    }

    /// Height of the subtree at `id` (1 for a leaf), iteratively.
    pub(crate) fn height(&self, id: usize) -> usize {
        let mut best = 0;
        let mut stack = vec![(id, 1usize)];
        let mut owned = Vec::new();
        while let Some((n, h)) = stack.pop() {
            best = best.max(h);
            self.push_owned(n, &mut owned);
            stack.extend(owned.drain(..).map(|c| (c, h + 1)));
        }
        best
    }

    /// Fails if a subtree of `height` levels can't go under `parent`.
    pub(crate) fn check_depth(&self, parent: usize, height: usize) -> Result<(), DomError> {
        if self.depth(parent) + height > self.limits.max_depth {
            return Err(DomError::TreeTooDeep {
                limit: self.limits.max_depth,
            });
        }
        Ok(())
    }

    /// Detaches `id` from its parent; the subtree stays alive.
    pub(crate) fn detach(&mut self, id: usize) {
        if let Some(parent) = self.node(id).parent {
            if self.is_observed() {
                let previous = self.sibling(id, false);
                let next = self.sibling(id, true);
                self.note_child_list(parent, &[], &[id], previous, next);
            }
            self.version += 1;
            self.inner.mutate().remove_node(id);
        }
    }

    /// Puts `nodes`, which have no parent, under `parent`, before `before`
    /// or last. Only parentless nodes: blitz-dom's own move removes a node
    /// from the old parent's list by id after adding it to the new one, so
    /// moving within one parent would lose the node and leave a parent link
    /// nothing points back to.
    pub(crate) fn place(&mut self, parent: usize, nodes: &[usize], before: Option<usize>) {
        if self.is_observed() && !nodes.is_empty() {
            let previous = match before {
                Some(b) => self.sibling(b, false),
                None => self.children(parent).last().copied(),
            };
            self.note_child_list(parent, nodes, &[], previous, before);
        }
        self.version += 1;
        if self.is_in_style(parent) {
            for &n in nodes {
                self.stash_text(n);
            }
        }
        match before {
            Some(b) => self.inner.mutate().insert_nodes_before(b, nodes),
            None => self.inner.mutate().append_children(parent, nodes),
        }
    }

    fn is_observed(&self) -> bool {
        !self.registrations.is_empty()
    }

    /// DOM §4.3 "queue a mutation record": `wanted` picks the registrations
    /// on `target` and its ancestors that are interested; each observer once,
    /// with the old value if any of its registrations asked. `old_value`
    /// runs only if one did.
    fn queue_record(
        &mut self,
        target: usize,
        wanted: impl Fn(&Registration) -> Option<bool>,
        mut record: impl FnMut(&mut Self, bool) -> Option<MutationRecord>,
    ) {
        let mut interested: Vec<(u32, bool)> = Vec::new();
        let mut at = Some(target);
        while let Some(n) = at {
            for r in self.registrations.get(&n).into_iter().flatten() {
                if n != target && r.flags & SUBTREE == 0 {
                    continue;
                }
                let Some(old) = wanted(r) else {
                    continue;
                };
                match interested.iter_mut().find(|(o, _)| *o == r.observer) {
                    Some((_, o)) => *o |= old,
                    None => interested.push((r.observer, old)),
                }
            }
            at = self.node(n).parent;
        }
        for (observer, old) in interested {
            let Some(mut r) = record(self, old) else {
                return;
            };
            if !old {
                r.old_value = None;
            }
            match self.new_handle(Object::Record(Box::new(r))) {
                Ok(h) => self.record_queues[observer as usize].push(h),
                Err(e) => {
                    self.fault.get_or_insert(e);
                    return;
                }
            }
        }
    }

    /// The one empty static list records share. A failure is the call's
    /// fault, like every other record allocation.
    fn empty_list(&mut self) -> Option<Handle> {
        if self.empty_list == 0 {
            match self.static_list(Vec::new()) {
                Ok(h) => self.empty_list = h,
                Err(e) => {
                    self.fault.get_or_insert(e);
                    return None;
                }
            }
        }
        Some(self.empty_list)
    }

    fn pin(&mut self, id: Option<usize>) -> Option<usize> {
        if let Some(n) = id {
            if let Err(e) = self.handle_for(n) {
                self.fault.get_or_insert(e);
            }
        }
        id
    }

    fn note_child_list(
        &mut self,
        parent: usize,
        added: &[usize],
        removed: &[usize],
        previous: Option<usize>,
        next: Option<usize>,
    ) {
        self.queue_record(
            parent,
            |r| (r.flags & CHILD_LIST != 0).then_some(false),
            |doc, _| {
                let previous = doc.pin(previous);
                let next = doc.pin(next);
                let target = doc.pin(Some(parent))?;
                let added = doc.static_list(added.to_vec());
                let removed = doc.static_list(removed.to_vec());
                let (added, removed) = match (added, removed) {
                    (Ok(a), Ok(r)) => (a, r),
                    (Err(e), _) | (_, Err(e)) => {
                        doc.fault.get_or_insert(e);
                        return None;
                    }
                };
                Some(MutationRecord {
                    kind: "childList",
                    target,
                    added,
                    removed,
                    previous,
                    next,
                    attribute: None,
                    namespace: None,
                    old_value: None,
                })
            },
        );
    }

    fn note_attribute(&mut self, id: usize, name: &QualName) {
        if !self.is_observed() {
            return;
        }
        let local = name.local.to_string();
        self.queue_record(
            id,
            |r| {
                let listed =
                    r.flags & HAS_FILTER == 0 || (name.ns == ns!() && r.filter.contains(&local));
                (r.flags & ATTRIBUTES != 0 && listed).then_some(r.flags & ATTRIBUTE_OLD != 0)
            },
            |doc, _| {
                let empty = doc.empty_list()?;
                let target = doc.pin(Some(id))?;
                Some(MutationRecord {
                    kind: "attributes",
                    target,
                    added: empty,
                    removed: empty,
                    previous: None,
                    next: None,
                    attribute: Some(local.clone()),
                    namespace: Some(name.ns.to_string()).filter(|n| !n.is_empty()),
                    old_value: doc.attribute_value(id, name),
                })
            },
        );
    }

    fn note_character_data(&mut self, id: usize) {
        if !self.is_observed() {
            return;
        }
        self.queue_record(
            id,
            |r| (r.flags & CHARACTER_DATA != 0).then_some(r.flags & CHARACTER_OLD != 0),
            |doc, _| {
                let empty = doc.empty_list()?;
                let target = doc.pin(Some(id))?;
                Some(MutationRecord {
                    kind: "characterData",
                    target,
                    added: empty,
                    removed: empty,
                    previous: None,
                    next: None,
                    attribute: None,
                    namespace: None,
                    old_value: Some(doc.char_data(id).to_owned()),
                })
            },
        );
    }

    pub(crate) fn new_observer(&mut self) -> u32 {
        self.record_queues.push(Vec::new());
        u32::try_from(self.record_queues.len() - 1).unwrap_or(u32::MAX)
    }

    /// `observe`: a second call on one node replaces the first (§4.3.1).
    pub(crate) fn add_registration(
        &mut self,
        observer: u32,
        target: usize,
        flags: u32,
        names: &str,
    ) {
        let filter = if names.is_empty() {
            Vec::new()
        } else {
            names.split('\n').map(str::to_owned).collect()
        };
        let list = self.registrations.entry(target).or_default();
        let r = Registration {
            observer,
            flags,
            filter,
        };
        match list.iter_mut().find(|x| x.observer == observer) {
            Some(old) => *old = r,
            None => list.push(r),
        }
    }

    /// `disconnect`: no registrations, no queued records.
    pub(crate) fn remove_registrations(&mut self, observer: u32) {
        self.registrations.retain(|_, list| {
            list.retain(|r| r.observer != observer);
            !list.is_empty()
        });
        if let Some(q) = self.record_queues.get_mut(observer as usize) {
            q.clear();
        }
    }

    /// The observer's oldest queued record, if any.
    pub(crate) fn next_record(&mut self, observer: u32) -> Option<Handle> {
        let q = self.record_queues.get_mut(observer as usize)?;
        (!q.is_empty()).then(|| q.remove(0))
    }

    pub(crate) fn has_records(&self) -> bool {
        self.record_queues.iter().any(|q| !q.is_empty())
    }

    /// `<template>`'s content, if it has one yet.
    pub(crate) fn template_of(&self, id: usize) -> Option<usize> {
        self.templates.get(&id).copied()
    }

    /// `<template>`'s content, made on first use for one a script created.
    pub(crate) fn template_content(&mut self, id: usize) -> Result<usize, DomError> {
        if let Some(&c) = self.templates.get(&id) {
            return Ok(c);
        }
        let c = self.create_fragment()?;
        self.templates.insert(id, c);
        Ok(c)
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
