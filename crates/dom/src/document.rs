//! A parsed document and the handle table that scripts address it through.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use blitz_dom::node::NodeFlags;
use blitz_dom::{BaseDocument, DocumentConfig, LocalName, NodeData, QualName, ns};
use surimono_core::Deadline;
use surimono_webidl::runtime::{BindingError, BindingResult};

use crate::DomError;
use crate::parse::{self, ParseLimits};
use crate::style_attr;

/// Infra's ASCII whitespace.
const ASCII_WHITESPACE: [char; 5] = ['\t', '\n', '\x0c', '\r', ' '];

/// Whether `s` is a JavaScript MIME type essence string (MIME Sniffing §4.6:
/// "JavaScript MIME type essence match"), ignoring ASCII case.
fn is_javascript_mime_essence(s: &str) -> bool {
    const ESSENCES: [&str; 16] = [
        "application/ecmascript",
        "application/javascript",
        "application/x-ecmascript",
        "application/x-javascript",
        "text/ecmascript",
        "text/javascript",
        "text/javascript1.0",
        "text/javascript1.1",
        "text/javascript1.2",
        "text/javascript1.3",
        "text/javascript1.4",
        "text/javascript1.5",
        "text/jscript",
        "text/livescript",
        "text/x-ecmascript",
        "text/x-javascript",
    ];
    ESSENCES.iter().any(|e| e.eq_ignore_ascii_case(s))
}

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
    /// Most bytes of strings the document may hold: text, comment data,
    /// element names, and attribute names and values (with
    /// [`ATTRIBUTE_COST`] each), parsed and from scripts together, and the
    /// names and values script objects keep (live lists' names, removed
    /// `Attr`s). Counted on write and credited on overwrite and
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
    /// An `Attr`: its element, its name, and, once it has left the element,
    /// its value (charged, see [`Document::retained_bytes`]).
    Attr(usize, QualName, Option<String>),
}

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

/// Steps of a native's loop between looks at the clock.
const POLL_EVERY: u32 = 256;

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
    /// Bytes held in `objects` and `owned` rather than in nodes: list names
    /// (kept as long as the document, for `[SameObject]`-style identity) and
    /// the values of removed `Attr`s. Part of `held_bytes`.
    retained_bytes: usize,
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
    /// The running call's deadline, set for each native call.
    pub(crate) deadline: Option<Deadline>,
    /// Steps since the last look at the clock (see [`Document::poll`]).
    ticks: Cell<u32>,
    /// Entries of child lists passed by changes (see [`Document::list_steps`]).
    list_steps: u64,
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
    /// Fails with [`DomError::NodeLimit`], [`DomError::TreeTooDeep`] or
    /// [`DomError::TextLimit`] if the parsed tree is already over `limits`
    /// (html5ever doesn't cap nesting), and with [`DomError::InvalidLimits`] for limits out of
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
            retained_bytes: 0,
            node_to_handle: Vec::new(),
            side_data: parsed.strings.into_iter().collect(),
            held_bytes: 0,
            quirks: parsed.quirks,
            ready_state: 2,
            cursor: Cell::new((0, 0)),
            fault: None,
            deadline: None,
            ticks: Cell::new(0),
            list_steps: 0,
        };
        if doc.node_count() > limits.max_nodes {
            return Err(DomError::NodeLimit {
                limit: limits.max_nodes,
            });
        }
        if parsed.over_style {
            return Err(style_attr::limit_error());
        }
        if doc.height(doc.root()) - 1 > limits.max_depth || parsed.over_limit {
            return Err(DomError::TreeTooDeep {
                limit: limits.max_depth,
            });
        }
        // Every node, not only those under the root: orphans count until
        // they go, so a page can't hide bytes in them.
        doc.held_bytes = doc.bytes_held();
        if doc.held_bytes > limits.max_text_bytes {
            return Err(DomError::TextLimit {
                limit: limits.max_text_bytes,
            });
        }
        for orphan in parsed.removed {
            doc.drop_orphan(orphan);
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
            || deadline.as_ref().is_some_and(Deadline::passed),
        );
        self.side_data.extend(parsed.strings);
        // Charged here and credited by the drops below if it is refused. The
        // holder is charged already. Orphans count until they go.
        let bytes = self.bytes_under(holder) - self.node_bytes(holder);
        self.held_bytes = self.held_bytes.saturating_add(bytes);
        for orphan in parsed.removed {
            if self
                .inner
                .get_node(orphan)
                .is_some_and(|n| n.parent.is_none())
            {
                self.held_bytes = self.held_bytes.saturating_add(self.bytes_under(orphan));
                self.drop_orphan(orphan);
            }
        }
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
        let timed_out = deadline.as_ref().is_some_and(Deadline::passed);
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
        if parsed.over_style {
            self.drop_subtree(holder);
            return Err(style_attr::limit_error());
        }
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
        for &c in self.children(id) {
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

    /// One step of a loop whose length a script controls, inside a native.
    /// The sandbox can't interrupt a native, so such a loop ends itself:
    /// past the deadline this is an exception. As at the start of a native,
    /// no fault is recorded: the engine stops the script at its next check,
    /// so the call fails with `TimeLimit` even if the script catches this.
    pub(crate) fn poll(&self) -> BindingResult<()> {
        self.poll_by(1)
    }

    /// [`poll`](Self::poll) for a step that does `work` units of work.
    pub(crate) fn poll_by(&self, work: usize) -> BindingResult<()> {
        let work = u32::try_from(work).unwrap_or(u32::MAX);
        let ticks = self.ticks.get().saturating_add(work);
        if ticks < POLL_EVERY {
            self.ticks.set(ticks);
            return Ok(());
        }
        self.ticks.set(0);
        if self.deadline.as_ref().is_some_and(Deadline::passed) {
            return Err(BindingError::TypeError(
                "surimono: the time limit has passed".to_owned(),
            ));
        }
        Ok(())
    }

    /// The document's serialization: the `<html>` element's outer HTML, by
    /// the HTML Standard's fragment serialization algorithm (§13.3) for
    /// elements (HTML, SVG and MathML) with their attributes, text and
    /// comments, with scripting enabled, as the document was parsed.
    /// Iterative, so tree depth can't overflow the stack.
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
                            // node, as it always is here (see `parse::opts`).
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
                                                | "noscript"
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

    fn element_mut(&mut self, id: usize) -> Option<&mut blitz_dom::ElementData> {
        self.inner
            .get_node_mut(id)
            .and_then(|n| n.element_data_mut())
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
        // Lists and `Attr`s keep their names for the document's life, here
        // and in the object: strings a script picks, outside the JS heap.
        let bytes = match &key {
            Owned::List(ListKind::ByTag(s) | ListKind::ByClass(s)) => s.len().saturating_mul(2),
            Owned::Attr(name) => name_cost(name),
            _ => 0,
        };
        self.recharge(0, bytes)?;
        self.retained_bytes += bytes;
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
    pub(crate) fn live_nodes(
        &mut self,
        handle: Handle,
        owner: usize,
        kind: &ListKind,
    ) -> BindingResult<&[usize]> {
        let fresh = self
            .list_cache
            .as_ref()
            .is_some_and(|(h, v, _)| *h == handle && *v == self.version);
        if !fresh {
            let nodes = self.compute_list(owner, kind)?;
            self.list_cache = Some((handle, self.version, nodes));
        }
        Ok(self
            .list_cache
            .as_ref()
            .map_or(&[], |(_, _, n)| n.as_slice()))
    }

    /// Descendant elements of `top` in tree order that satisfy `keep`.
    pub(crate) fn descendants(
        &self,
        top: usize,
        mut keep: impl FnMut(usize) -> BindingResult<bool>,
    ) -> BindingResult<Vec<usize>> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.children(top).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            self.poll()?;
            if self.kind(n) == Some(Kind::Element) && keep(n)? {
                out.push(n);
            }
            stack.extend(self.children(n).iter().rev().copied());
        }
        Ok(out)
    }

    fn compute_list(&self, owner: usize, kind: &ListKind) -> BindingResult<Vec<usize>> {
        Ok(match kind {
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
                        return Ok(false);
                    };
                    // DOM §4.2.6.2: lower-cased against HTML elements.
                    let wanted = if e.ns == ns!(html) { &lower } else { name };
                    Ok(name == "*" || qualified_is(e, wanted))
                })?
            }
            ListKind::ByClass(names) => self.elements_with_classes(owner, names)?,
        })
    }

    /// `getElementsByClassName`: linear in the names and in the classes
    /// read, however many of each there are. Only looked up, so hash order
    /// can't reach any output.
    fn elements_with_classes(&self, owner: usize, names: &str) -> BindingResult<Vec<usize>> {
        // Case-insensitive in quirks mode (DOM §4.2.6.2).
        let folded = |s| {
            if self.quirks {
                Cow::Owned(str::to_ascii_lowercase(s))
            } else {
                Cow::Borrowed(s)
            }
        };
        let mut wanted: HashMap<Cow<'_, str>, usize> = HashMap::new();
        for name in names.split_ascii_whitespace() {
            self.poll()?;
            let next = wanted.len();
            wanted.entry(folded(name)).or_insert(next);
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        // The last element each name was seen on, as a count, so a name
        // repeated in one `class` isn't counted twice.
        let mut seen = vec![0usize; wanted.len()];
        let mut element = 0;
        self.descendants(owner, |n| {
            let Some(class) = self.find_attribute(n, "class") else {
                return Ok(false);
            };
            self.poll_by(class.value.len() / 16)?;
            element += 1;
            let mut found = 0;
            for token in class.value.split_ascii_whitespace() {
                if let Some(&i) = wanted.get(folded(token).as_ref())
                    && seen[i] != element
                {
                    seen[i] = element;
                    found += 1;
                }
            }
            Ok(found == wanted.len())
        })
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

    /// Whether `id` is an HTML element with one of these local names.
    fn is_html_element(&self, id: usize, locals: &[&str]) -> bool {
        self.node(id)
            .element_data()
            .is_some_and(|e| e.name.ns == ns!(html) && locals.contains(&e.name.local.as_ref()))
    }

    /// The first child of the html element (HTML "the html element": the
    /// document element, if it is one) with one of these local names.
    fn html_child(&self, locals: &[&str]) -> Option<usize> {
        let html = self
            .document_element()
            .filter(|&e| self.is_html_element(e, &["html"]))?;
        let mut children = self.node(html).children.iter().copied();
        children.find(|&c| self.is_html_element(c, locals))
    }

    pub(crate) fn head(&self) -> Option<usize> {
        self.html_child(&["head"])
    }

    /// HTML "the body element": the first body or frameset child.
    pub(crate) fn body(&self) -> Option<usize> {
        self.html_child(&["body", "frameset"])
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
    /// value, text, defer), as HTML §4.12.1.1 "prepare the script element"
    /// picks them. `module` and other types don't run. A parser-inserted
    /// `async` script keeps its tree position, which is one of the orders
    /// the spec allows and the deterministic one. Nothing inside `<template>`
    /// runs: its contents are inert. `<noscript>` needs no such care: with
    /// scripting on it is raw text, so the parser makes no script there.
    pub(crate) fn scripts(&self) -> Vec<(usize, Option<String>, String, bool)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            let html_name = self
                .element_name(n)
                .filter(|q| q.ns == ns!(html))
                .map(|q| &*q.local);
            if html_name == Some("template") {
                continue;
            }
            stack.extend(self.children(n).iter().rev().copied());
            if html_name != Some("script") || !self.is_classic_script(n) {
                continue;
            }
            let src = self.attribute(n, "src");
            let text = self.child_text(n);
            if src.is_none() && text.is_empty() {
                continue;
            }
            let defer = src.is_some()
                && self.attribute(n, "defer").is_some()
                && self.attribute(n, "async").is_none();
            out.push((n, src, text, defer));
        }
        out
    }

    /// HTML §4.12.1.1 "prepare the script element", the steps that give the
    /// type string, for a classic script. `nomodule` is ignored, unlike the
    /// standard: modules don't run here, so the fallback is what a page has
    /// for us (a recorded departure).
    fn is_classic_script(&self, id: usize) -> bool {
        let ty = self.attribute(id, "type");
        let language = self.attribute(id, "language");
        match (ty.as_deref(), language.as_deref()) {
            (Some(""), _) | (None, None | Some("")) => true,
            (Some(ty), _) => is_javascript_mime_essence(ty.trim_matches(ASCII_WHITESPACE)),
            (None, Some(language)) => is_javascript_mime_essence(&format!("text/{language}")),
        }
    }

    /// A node's child text content (DOM §4.2.5): its Text children, joined.
    fn child_text(&self, id: usize) -> String {
        self.children(id)
            .iter()
            .filter(|&&c| matches!(self.node(c).data, NodeData::Text(_)))
            .map(|&c| self.char_data(c))
            .collect()
    }

    /// The `<title>` element: the first in tree order (HTML §3.1.3).
    fn title_element(&self) -> BindingResult<Option<usize>> {
        let Some(top) = self.document_element() else {
            return Ok(None);
        };
        let is_title = |n: usize| {
            self.element_name(n)
                .is_some_and(|q| q.ns == ns!(html) && q.local.as_ref() == "title")
        };
        if is_title(top) {
            return Ok(Some(top));
        }
        let found = self.descendants(top, |n| Ok(is_title(n)))?;
        Ok(found.first().copied())
    }

    /// `document.title`: the title's child text content, with whitespace
    /// stripped and collapsed (HTML §3.1.3).
    pub(crate) fn title(&self) -> BindingResult<String> {
        Ok(self
            .title_element()?
            .map(|t| self.child_text(t))
            .map(|t| t.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default())
    }

    /// `document.title` set: the title's content, a new `<title>` in the
    /// head if there is none; nothing without a head.
    pub(crate) fn set_title(&mut self, value: &str) -> BindingResult<()> {
        let title = match self.title_element()? {
            Some(t) => t,
            None => {
                let Some(head) = self.head() else {
                    return Ok(());
                };
                let created = self
                    .create_element("title")
                    .and_then(|t| t.ok_or(DomError::NoDocument));
                let t = created.map_err(|e| self.fail(e))?;
                self.place(head, &[t], None);
                t
            }
        };
        self.set_text_content(title, value)
            .map_err(|e| self.fail(e))
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
        // Any namespace: blitz-dom goes by the local name.
        if name.local.as_ref() == "style" && !style_attr::fits(value) {
            return Err(style_attr::limit_error());
        }
        let credit = self.attribute_bytes(id, &name);
        let charge = attribute_cost(&name, value);
        self.recharge(credit, charge)?;
        self.version += 1;
        if name.local.as_ref() == "style" {
            // Not through blitz-dom, which would give it to stylo in a call
            // nothing can stop, for a layout that doesn't run here.
            if let Some(e) = self.element_mut(id) {
                e.attrs.set(name, value);
            }
        } else {
            self.inner.mutate().set_attribute(id, name, value);
        }
        Ok(())
    }

    /// An `Attr` object for an attribute about to go keeps its value.
    fn keep_attr_value(&mut self, id: usize, name: &QualName) {
        // Out of the map: if the name comes back, it's a new attribute.
        let Some(h) = self.owned.remove(&(id, Owned::Attr(name.clone()))) else {
            return;
        };
        let value = self.attribute_value(id, name).unwrap_or_default();
        // The attribute's own charge is credited next and covers this.
        self.held_bytes = self.held_bytes.saturating_add(value.len());
        self.retained_bytes += value.len();
        if let Some(Object::Attr(_, _, kept)) = self.object_mut(h) {
            *kept = Some(value);
        }
    }

    /// Writes the value of `handle`, an `Attr` no longer on its element.
    pub(crate) fn set_kept_value(&mut self, handle: Handle, value: String) -> Result<(), DomError> {
        let Some(Object::Attr(_, _, Some(kept))) = self.object_mut(handle) else {
            return Ok(());
        };
        let old = kept.len();
        self.recharge(old, value.len())?;
        self.retained_bytes = self.retained_bytes - old + value.len();
        if let Some(Object::Attr(_, _, kept)) = self.object_mut(handle) {
            *kept = Some(value);
        }
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
                .sum::<usize>()
                .saturating_add(name_cost(&e.name)),
            NodeData::Document => 0,
        }
    }

    /// Bytes every node holds, reachable or not, and the script objects'.
    fn bytes_held(&self) -> usize {
        self.inner
            .tree()
            .iter()
            .fold(self.retained_bytes, |sum, (id, _)| {
                sum.saturating_add(self.node_bytes(id))
            })
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

    /// Frees `id` and its subtree if the parser left it with no parent.
    fn drop_orphan(&mut self, id: usize) {
        if self.inner.get_node(id).is_some_and(|n| n.parent.is_none()) {
            self.drop_subtree(id);
        }
    }

    fn credit(&mut self, bytes: usize) {
        debug_assert!(self.held_bytes >= bytes, "a byte count went wrong");
        self.held_bytes = self.held_bytes.saturating_sub(bytes);
    }

    /// Swaps `credit` held bytes for `charge`, failing with
    /// [`DomError::TextLimit`] (and changing nothing) if that's over the
    /// limit. Callers charge before they write.
    fn recharge(&mut self, credit: usize, charge: usize) -> Result<(), DomError> {
        debug_assert!(self.held_bytes >= credit, "a byte count went wrong");
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
        self.version += 1;
        self.credit(self.attribute_bytes(id, &name));
        let is_link_href = self
            .element_name(id)
            .is_some_and(|n| n.local.as_ref() == "link")
            && name.local.as_ref() == "href";
        // blitz-dom's clear_attribute unloads the link's stylesheet here and
        // hits `unreachable!` if it has none, which it never does: nothing
        // loads. Nothing else depends on this attribute. For `style`, it would
        // parse what is left: it goes by the local name, of which an element
        // can have several, in different namespaces.
        if is_link_href || name.local.as_ref() == "style" {
            if let Some(e) = self.element_mut(id) {
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
        let gone = self.children(id).to_vec();
        self.replace_child_list(id, Vec::new(), gone);
    }

    /// Makes `kept` the children of `id` and removes `gone`, its other
    /// children, as [`remove_all_children`](Self::remove_all_children) does:
    /// one pass, however wide the list.
    pub(crate) fn replace_child_list(&mut self, id: usize, kept: Vec<usize>, gone: Vec<usize>) {
        self.version += 1;
        self.count_list_steps(kept.len() + gone.len());
        self.take_children(id);
        for c in gone {
            self.set_parent(c, None);
            if self.subtree_has_handle(c) {
                // No parent to update: this only updates blitz-dom's
                // document state for the subtree.
                self.inner.mutate().remove_node(c);
            } else {
                self.drop_subtree(c);
            }
        }
        // `id` has no children yet; this does blitz-dom's restyle
        // bookkeeping for it.
        self.inner.mutate().remove_and_drop_all_children(id);
        if let Some(node) = self.inner.get_node_mut(id) {
            node.children = kept;
        }
    }

    fn subtree_has_handle(&self, top: usize) -> bool {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            if self.has_handle(n) {
                return true;
            }
            stack.extend(self.node(n).children.iter().copied());
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
            self.credit(self.node_bytes(n));
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
        let name = QualName::new(None, ns!(), LocalName::from(FRAGMENT_NAME));
        self.recharge(0, name_cost(&name))?;
        Ok(self.inner.mutate().create_element(name, Vec::new()))
    }

    /// A new element with `name` and copies of `attrs`.
    pub(crate) fn create_element_with(
        &mut self,
        name: QualName,
        mut attrs: Vec<blitz_dom::Attribute>,
    ) -> Result<usize, DomError> {
        self.check_room()?;
        let attr_bytes: usize = attrs
            .iter()
            .map(|a| attribute_cost(&a.name, &a.value))
            .sum();
        self.recharge(0, attr_bytes.saturating_add(name_cost(&name)))?;
        let mut hidden = false;
        for a in &mut attrs {
            if a.name.local.as_ref() == "style" {
                style_attr::hide(&mut a.name);
                hidden = true;
            }
        }
        let id = self.inner.mutate().create_element(name, attrs);
        if hidden && let Some(e) = self.element_mut(id) {
            style_attr::reveal(&mut e.attrs);
        }
        Ok(id)
    }

    /// `createElement` for an HTML element; `None` for a name the DOM
    /// rejects with InvalidCharacterError.
    pub(crate) fn create_element(&mut self, name: &str) -> Result<Option<usize>, DomError> {
        if !is_valid_element_name(name) {
            return Ok(None);
        }
        self.check_room()?;
        let lower = name.to_ascii_lowercase();
        // Before the atom exists: it holds the whole name, in Rust.
        self.recharge(0, html_name_cost(&lower))?;
        let qual = QualName::new(None, ns!(html), LocalName::from(lower));
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
        while let Some((n, h)) = stack.pop() {
            best = best.max(h);
            stack.extend(self.node(n).children.iter().map(|&c| (c, h + 1)));
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
            self.version += 1;
            self.count_list_steps(self.children(parent).len());
            self.inner.mutate().remove_node(id);
        }
    }

    /// Puts `nodes`, which have no parent, under `parent`, before `before`
    /// or last. Only parentless nodes: blitz-dom's own move removes a node
    /// from the old parent's list by id after adding it to the new one, so
    /// moving within one parent would lose the node and leave a parent link
    /// nothing points back to.
    pub(crate) fn place(&mut self, parent: usize, nodes: &[usize], before: Option<usize>) {
        self.version += 1;
        self.count_list_steps(nodes.len() + before.map_or(0, |_| self.children(parent).len()));
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

    fn count_list_steps(&mut self, entries: usize) {
        self.list_steps = self.list_steps.saturating_add(entries as u64);
    }

    /// Entries of child lists passed so far by changes to the tree: a scan
    /// of a parent's list counts its length. A deterministic measure of what
    /// a change costs, so a test of "one pass, not one scan per child" doesn't
    /// compare times on a loaded machine.
    #[doc(hidden)]
    pub fn list_steps(&self) -> u64 {
        self.list_steps
    }

    /// Checks the tree's links and the byte count; for tests. Returns what
    /// is wrong, if anything.
    #[doc(hidden)]
    pub fn check_consistency(&self) -> Result<(), String> {
        let held = self.bytes_held();
        for (id, node) in self.inner.tree().iter() {
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

/// What a name is charged: all its atoms, which a script can make any size.
fn name_cost(name: &QualName) -> usize {
    name.local.len() + name.ns.len() + name.prefix.as_ref().map_or(0, |p| p.len())
}

/// [`name_cost`] of an HTML element called `local`, before its name exists.
fn html_name_cost(local: &str) -> usize {
    local.len() + ns!(html).len()
}

/// What one attribute is charged.
fn attribute_cost(name: &QualName, value: &str) -> usize {
    ATTRIBUTE_COST + name_cost(name) + value.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DomRuntime;
    use surimono_core::Limits;

    /// Elements with a block stylo parsed.
    fn parsed_blocks(doc: &Document) -> usize {
        (0..1000)
            .filter_map(|id| doc.inner.get_node(id))
            .filter_map(|n| n.element_data())
            .filter(|e| e.style_attribute.is_some())
            .count()
    }

    /// blitz-dom would parse each of these in a call nothing can stop.
    #[test]
    fn no_path_gives_a_style_attribute_to_stylo() {
        let page = r#"<!doctype html><html style="color: red"><body style="color: red">
            <p id="p" style="color: red"></p><template><b style="color: red"></b></template>
            <body style="top: 0" class="x"><html style="top: 0" lang="ja"><b><i style="top: 0">
            <p>x"#;
        let mut rt = DomRuntime::new(Limits::default()).unwrap();
        rt.load(Document::parse(page).unwrap());
        let got: String = rt
            .eval(
                r#"const p = document.getElementById('p');
                const body = document.body, xlink = 'http://www.w3.org/1999/xlink';
                const out = [document.documentElement.getAttribute('style'),
                    body.getAttribute('style'), [...body.attributes].map(a => a.name).join()];
                p.setAttribute('style', 'top: 1px');
                p.setAttributeNS(xlink, 'xlink:style', 'top: 2px');
                p.style.left = '3px';
                p.attributes.getNamedItem('style').value += 'right: 4px';
                p.innerHTML = '<b style="top: 5px"></b>';
                out.push(p.firstChild.outerHTML);
                p.appendChild(p.cloneNode(true));
                p.removeAttribute('style');
                out.push(p.getAttributeNS(xlink, 'style'), p.lastChild.getAttribute('style'));
                out.join('|')"#,
            )
            .unwrap();
        assert_eq!(
            got,
            "color: red|color: red|style,class|<b style=\"top: 5px\"></b>|top: 2px|\
             top: 1px; left: 3px;right: 4px"
        );
        let mut doc = rt.take().unwrap();
        assert_eq!(parsed_blocks(&doc), 0);
        // The check sees one when there is one.
        let id = doc.inner.root_element().id;
        let name = QualName::new(None, ns!(), LocalName::from("style"));
        doc.inner.mutate().set_attribute(id, name, "top: 0");
        assert_eq!(parsed_blocks(&doc), 1);
    }

    #[test]
    fn a_style_attribute_keeps_its_place_and_namespace() {
        let doc = Document::parse(r#"<p id="a" style="top: 0" class="b">"#).unwrap();
        let html = doc.to_html();
        assert!(
            html.contains(r#"<p id="a" style="top: 0" class="b">"#),
            "{html}"
        );
    }
}
