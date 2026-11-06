//! A parsed document and the handle table that scripts address it through.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use blitz_dom::node::NodeFlags;
use blitz_dom::{BaseDocument, DocumentConfig, LocalName, NodeData, QualName, ns};
use html5ever::Namespace;
use surimono_core::Deadline;
use surimono_webidl::runtime::{BindingError, BindingResult};

use crate::DomError;
use crate::parse::{self, ParseLimits};
use crate::range::LiveRange;
use crate::style_attr;
use crate::xml;

/// Most `<iframe>`s loaded in a render. Each costs a parse, and a page can
/// make them in a loop; later ones stay at `about:blank`, with no `load`.
const MAX_FRAMES: usize = 64;

/// The XML `contentType` of a frame at `url`, by its path's extension, or
/// `None` for HTML.
fn frame_content_type(url: &str) -> Option<&'static str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path.rsplit('/').next()?.rsplit_once('.')?.1;
    match extension {
        "xht" | "xhtml" => Some(APPLICATION_XHTML),
        "xml" => Some(APPLICATION_XML),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

/// The longest URL a script can give a document: one string in one call, on
/// top of what `max_text_bytes` charges for all of them. Chrome takes longer
/// ones; nothing a page prints needs them.
pub(crate) const MAX_URL_BYTES: usize = 2 * 1024 * 1024;

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
    /// Most attributes on one element. html5ever's cost for a tag is
    /// quadratic in its attributes, and so is any tree's for an element: a
    /// page has a few dozen.
    pub max_attributes: usize,
    /// Most `Range`s made over the document's life. A range can't be known
    /// to be unreachable, so it stays live until the document drops, and
    /// mutations may have to look at all of them.
    pub max_ranges: usize,
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
    /// 4,096 attributes, the printing pass's cap.
    pub const DEFAULT_MAX_ATTRIBUTES: usize = 4_096;
    /// 65,536 ranges: a page that measures each label through a range of its
    /// own, on a large chart, makes thousands.
    pub const DEFAULT_MAX_RANGES: usize = 65_536;

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
            || self.max_attributes == 0
            || self.max_ranges == 0
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
            max_attributes: Self::DEFAULT_MAX_ATTRIBUTES,
            max_ranges: Self::DEFAULT_MAX_RANGES,
        }
    }
}

/// What a script's layout queries do until phase 3 answers them from real
/// layout: `getBoundingClientRect`, `getClientRects`, `client*`, `scroll*`,
/// `offset*`, `elementFromPoint`, `getComputedStyle` and the window's
/// `innerWidth`, `outerWidth`, `scrollX` and the like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayoutPolicy {
    /// Answer as a browser does for an element with no box: zeros, an empty
    /// rectangle list, a null `offsetParent` and `elementFromPoint`, and a
    /// computed style with no properties. Pages that only measure keep going.
    #[default]
    Zeros,
    /// End the call with [`DomError::LayoutQuery`], for callers who would
    /// rather have no page than one built on made-up measurements.
    Throw,
}

/// What kind of node a handle names, as the DOM's `nodeType` numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Element = 1,
    /// Text, and `CDATASection` (`is_cdata`), which is a Text node.
    Text = 3,
    ProcessingInstruction = 7,
    Comment = 8,
    Document = 9,
    DocumentType = 10,
    DocumentFragment = 11,
}

/// What blitz-dom has no node for. A doctype and a processing instruction are
/// comment nodes to it, so it and its layout skip them; a CDATA section is a
/// text node. The kind is here, by node id. Only looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Special {
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    /// Its data is in `side_data`, as a comment's is.
    Instruction {
        target: String,
    },
    Cdata,
}

impl Special {
    /// Strings it holds, charged to the document.
    fn bytes(&self) -> usize {
        match self {
            Self::Doctype {
                name,
                public_id,
                system_id,
            } => name.len() + public_id.len() + system_id.len(),
            Self::Instruction { target } => target.len(),
            Self::Cdata => 0,
        }
    }
}

/// `contentType` of an HTML document.
pub(crate) const TEXT_HTML: &str = "text/html";
/// `contentType` of an XML document with no better type.
pub(crate) const APPLICATION_XML: &str = "application/xml";
const APPLICATION_XHTML: &str = "application/xhtml+xml";

/// What differs between the documents of a render (DOM §4.5).
#[derive(Debug, Clone)]
pub(crate) struct DocInfo {
    /// Its type is HTML, not XML (DOM "HTML document").
    pub html: bool,
    /// The interface is `XMLDocument`, not `Document`.
    pub xml_interface: bool,
    pub quirks: bool,
    pub content_type: &'static str,
    /// An iframe's: a `defaultView`, though no script of its own.
    pub frame: bool,
    /// That iframe, for the walk up to the page. It may have a newer document
    /// by now: `frames` says.
    pub frame_element: Option<usize>,
    /// One allocation, however many documents take the URL.
    pub url: Arc<str>,
    /// `url` came from a script, and is charged to `max_text_bytes`.
    pub url_charged: bool,
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

/// The element name that marks a document other than the slab's first: a
/// blitz-dom document has one document node. All documents of a render live in
/// one slab, so a node moves between them by a change of owner, a wrapper
/// stays valid when it does, and one set of limits covers them all.
const DOCUMENT_NAME: &str = "#document";

/// Whether `name` is the marker of a fragment or of a document, which no
/// script sees as an element.
pub(crate) fn is_container_name(name: &QualName) -> bool {
    matches!(name.local.as_ref(), FRAGMENT_NAME | DOCUMENT_NAME)
}

/// Whether `name` is the marker of a document.
pub(crate) fn is_document_name(name: &QualName) -> bool {
    name.local.as_ref() == DOCUMENT_NAME
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
    /// `getElementsByTagNameNS`: namespace (`None` for `*`), local name.
    ByTagNs(Option<String>, String),
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
    /// A `Range`: its number in `Document::ranges`.
    Range(u32),
    /// A `StaticRange`, which never changes. Its nodes have handles.
    StaticRange(Box<LiveRange>),
    /// `document.implementation`.
    Implementation(usize),
    /// `getComputedStyle`: empty and read-only until there is layout.
    Computed(usize),
    /// A `MutationRecord`.
    Record(Box<MutationRecord>),
    /// An `Attr`: its element, its name, and, once it has left the element,
    /// its value (charged, see [`Document::retained_bytes`]).
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

impl Registration {
    /// What it is charged; see [`Document::add_registration`].
    fn cost(&self) -> usize {
        REGISTRATION_COST + self.filter.iter().map(String::len).sum::<usize>()
    }
}

/// What one registration is charged beyond its filter's strings: they live
/// in Rust, and one observer can be registered on every node.
pub const REGISTRATION_COST: usize = 64;

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
    Implementation,
    Computed,
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
    pub(crate) limits: DomLimits,
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
    /// Mutation observers: registrations by node, and each observer's queue
    /// of record handles, by observer number. Empty until a page observes,
    /// and mutations check that first.
    registrations: HashMap<usize, Vec<Registration>>,
    record_queues: Vec<VecDeque<Handle>>,
    /// Records in all queues: asked after every native call once a page
    /// observes, so it can't walk them.
    queued_records: usize,
    /// The part of `held_bytes` no node accounts for: registrations, and
    /// strings in mutation records. Records last as long as their handles,
    /// which is the document's life.
    observer_bytes: usize,
    /// See [`Document::empty_list`]; 0 until made.
    empty_list: Handle,
    /// `<template>` to its content fragment.
    templates: HashMap<usize, usize>,
    /// Blitz node id to its handle, 0 for none yet.
    node_to_handle: Vec<Handle>,
    /// Strings blitz-dom doesn't hold, by node id: comment data
    /// (`NodeData::Comment` has no room for it) and text under a `<style>`
    /// (see [`parse`](crate::parse)). Only looked up, so hash order can't
    /// reach any output.
    side_data: HashMap<usize, String>,
    /// Bytes held (see [`DomLimits::max_text_bytes`]).
    held_bytes: usize,
    /// Each document node's own state (see [`DocInfo`]).
    docs: HashMap<usize, DocInfo>,
    /// The document of every node not in the first one, documents themselves
    /// included (each is its own). Empty on an ordinary page, and looked at
    /// through that first. Only looked up.
    owners: HashMap<usize, usize>,
    /// The document nodes made now belong to: the document of the running
    /// member's `this` (see [`Document::enter`]).
    making: usize,
    /// The page's document is an HTML document: all of them are, on an
    /// ordinary page, which saves the lookup.
    page_html: bool,
    /// An `<iframe>` to its document, once it has one.
    frames: HashMap<usize, usize>,
    /// `<iframe>`s whose load has been started. Each loads once.
    frames_started: HashSet<usize>,
    /// Loads started in this render, whatever became of them.
    frame_count: usize,
    /// Sources a resolver gave, to parse inside a call: (element, text, URL).
    frame_sources: VecDeque<(usize, Option<String>, String)>,
    /// Handles of `<iframe>`s whose `load` is due.
    frame_loads: VecDeque<Handle>,
    /// See [`Special`].
    specials: HashMap<usize, Special>,
    /// Every `Range`, live until the document drops: see [`crate::range`].
    pub(crate) ranges: Vec<LiveRange>,
    /// Boundary points by node, counted, so a mutation of a node with none
    /// doesn't look at `ranges`. Only looked up.
    pub(crate) range_points: HashMap<usize, u32>,
    /// `readyState`: 0 loading, 1 interactive, 2 complete. A document no
    /// page lifecycle has run on is complete.
    ready_state: u32,
    /// The last (parent, index in its children) a sibling step found, so
    /// walking a wide parent's children isn't quadratic. Checked on use.
    pub(crate) cursor: Cell<(usize, usize)>,
    /// A fatal error a binding met, for the native call to report.
    pub(crate) fault: Option<DomError>,
    /// The running call's deadline, set for each native call.
    pub(crate) deadline: Option<Deadline>,
    /// What layout queries do, set for each native call.
    pub(crate) layout: LayoutPolicy,
    /// The body or frameset element whose `on*` attribute was written last:
    /// its attributes are the window's. A handle, so its node stays.
    pub(crate) handler_source: Option<Handle>,
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
    /// Parses `source` as an XML document (a page served as XHTML), with
    /// the default [`DomLimits`]. See [`parse_xml_with_limits`](Self::parse_xml_with_limits).
    pub fn parse_xml(source: &str) -> Result<Self, DomError> {
        Self::parse_xml_with_limits(source, DomLimits::default())
    }

    /// Parses `source` as an XML document of type `application/xhtml+xml`.
    /// Input that isn't well-formed gives a document with a `parsererror`
    /// element, as DOMParser's does, so the page's scripts run on what a
    /// browser would show. Fails on the same limits as [`parse_with_limits`](Self::parse_with_limits).
    pub fn parse_xml_with_limits(source: &str, limits: DomLimits) -> Result<Self, DomError> {
        Self::parse_xml_stoppable(source, limits, || false)
    }

    /// [`parse_xml_with_limits`](Self::parse_xml_with_limits) under a caller's
    /// stop check, asked as tokens go by: once `stop` says so, the parse
    /// fails with [`DomError::Stopped`] and frees what it made. This parse runs
    /// outside any script's time limit, as [`parse_stoppable`](Self::parse_stoppable)'s does.
    pub fn parse_xml_stoppable(
        source: &str,
        limits: DomLimits,
        stop: impl Fn() -> bool,
    ) -> Result<Self, DomError> {
        let mut doc = Self::parse_with_limits("", limits)?;
        let root = doc.root();
        doc.remove_children_quietly(root);
        if let Some(d) = doc.docs.get_mut(&root) {
            d.html = false;
            d.quirks = false;
            d.content_type = APPLICATION_XHTML;
        }
        doc.page_html = false;
        doc.fill_xml_until(root, source, &stop)?;
        Ok(doc)
    }

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
        Self::parse_stoppable(html, limits, || false)
    }

    /// [`parse_with_limits`](Self::parse_with_limits) under a caller's stop
    /// check, asked between feeds of the parser: once `stop` says so, the
    /// parse fails with [`DomError::Stopped`] and frees what it made. For input
    /// a caller doesn't trust, because this parse runs outside any script's
    /// time limit and one tag can cost quadratic time in its length until
    /// [`DomLimits::max_attributes`] can see it, at the tag's end.
    pub fn parse_stoppable(
        html: &str,
        limits: DomLimits,
        stop: impl Fn() -> bool,
    ) -> Result<Self, DomError> {
        limits.validate()?;
        // `stop` may answer differently next time; what it said is kept.
        let halted = Cell::new(false);
        let stop = || {
            let now = stop();
            halted.set(halted.get() || now);
            now
        };
        let mut inner = BaseDocument::new(DocumentConfig::default());
        // Nothing is "in the document" as far as blitz-dom can tell. Its
        // hooks for elements joining the document have no use here (nothing
        // loads, nothing lays out yet) and do harm: they panic on URLs they
        // can't resolve, and add text children to <input> outside `max_nodes`.
        inner
            .root_node_mut()
            .flags
            .remove(NodeFlags::IS_IN_DOCUMENT);
        let root = inner.root_node().id;
        let parsed = parse::parse_document(
            &mut inner.mutate(),
            root,
            html,
            ParseLimits {
                max_nodes: limits.max_nodes,
                max_depth: limits.max_depth,
                max_attributes: limits.max_attributes,
            },
            true,
            stop,
        );
        let info = DocInfo {
            url_charged: false,
            frame_element: None,
            html: true,
            xml_interface: false,
            quirks: parsed.quirks,
            content_type: TEXT_HTML,
            frame: false,
            url: Arc::from("about:blank"),
        };
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
            registrations: HashMap::new(),
            record_queues: Vec::new(),
            queued_records: 0,
            observer_bytes: 0,
            empty_list: 0,
            templates: parsed.templates.into_iter().collect(),
            node_to_handle: Vec::new(),
            side_data: parsed.strings.into_iter().collect(),
            held_bytes: 0,
            docs: HashMap::from([(root, info)]),
            owners: HashMap::new(),
            making: root,
            page_html: true,
            frames: HashMap::new(),
            frames_started: HashSet::new(),
            frame_count: 0,
            frame_sources: VecDeque::new(),
            frame_loads: VecDeque::new(),
            specials: parsed.specials.into_iter().collect(),
            ranges: Vec::new(),
            range_points: HashMap::new(),
            ready_state: 2,
            cursor: Cell::new((0, 0)),
            fault: None,
            deadline: None,
            layout: LayoutPolicy::default(),
            handler_source: None,
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
        if parsed.over_attributes {
            return Err(DomError::AttributeLimit {
                limit: limits.max_attributes,
            });
        }
        if halted.get() {
            return Err(DomError::Stopped);
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
        // HTML §13.4: the flag is that of the context's node document, and only
        // the page's has a browsing context.
        let scripting = self.making == self.root();
        let parsed = parse::parse_fragment(
            &mut self.inner.mutate(),
            context,
            holder,
            html,
            ParseLimits {
                max_nodes: room,
                max_depth: levels,
                max_attributes: self.limits.max_attributes,
            },
            scripting,
            || deadline.as_ref().is_some_and(Deadline::passed),
        );
        self.side_data.extend(parsed.strings);
        self.specials.extend(parsed.specials);
        // Before the count: a walk goes through a template to its content.
        self.templates.extend(parsed.templates);
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
        if parsed.over_attributes {
            self.drop_subtree(holder);
            return Err(DomError::AttributeLimit {
                limit: self.limits.max_attributes,
            });
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
        // The parser's nodes didn't go through `born`.
        if self.making != self.root() {
            self.own_subtree(holder, self.making);
        }
        Ok(Some(holder))
    }

    /// The document of the node `this` names, once `enter` has run: where the
    /// member's new nodes go.
    pub(crate) fn making(&self) -> usize {
        self.making
    }

    /// The start of a member's call on object `this`: nodes it makes join
    /// that object's document (DOM: "node document"). `new Text()` and other
    /// constructors come with the first document as `this`.
    pub(crate) fn enter(&mut self, this: Handle) {
        let root = self.root();
        self.making = root;
        if self.owners.is_empty() {
            return;
        }
        let owner = match this
            .checked_sub(1)
            .and_then(|i| self.objects.get(i as usize))
        {
            Some(
                Object::Node(n)
                | Object::Live(n, _)
                | Object::Tokens(n)
                | Object::Attributes(n)
                | Object::Dataset(n)
                | Object::Style(n)
                | Object::Implementation(n)
                | Object::Computed(n)
                | Object::Attr(n, ..),
            ) => *n,
            _ => root,
        };
        self.making = self.owner_of_node(owner);
    }

    /// `id`'s node document; a document is its own.
    pub(crate) fn owner_of_node(&self, id: usize) -> usize {
        self.owners.get(&id).copied().unwrap_or_else(|| self.root())
    }

    /// The state of document `doc`.
    pub(crate) fn info(&self, doc: usize) -> &DocInfo {
        // Every document node has an entry, made with it, and lives as long
        // as the slab; `doc` comes from `owners`, `making` or a `Kind::Document`.
        self.docs
            .get(&doc)
            .unwrap_or_else(|| &self.docs[&self.root()])
    }

    /// Whether `id` is in an HTML document, not an XML one.
    pub(crate) fn in_html_document(&self, id: usize) -> bool {
        if self.owners.is_empty() {
            return self.page_html;
        }
        self.info(self.owner_of_node(id)).html
    }

    fn set_owner(&mut self, id: usize, doc: usize) {
        if doc == self.root() {
            self.owners.remove(&id);
        } else {
            self.owners.insert(id, doc);
        }
    }

    /// DOM §4.5 "adopt", after the removal: `top` and what it owns go to
    /// `doc`. Linear in the subtree; nothing to do within one document.
    pub(crate) fn adopt_subtree(&mut self, top: usize, doc: usize) {
        if self.owner_of_node(top) != doc {
            self.own_subtree(top, doc);
            // What a live list answers depends on the document's type.
            self.version += 1;
        }
    }

    /// Makes `doc` the document of `top`, its subtree and its templates'
    /// content, whatever they had.
    fn own_subtree(&mut self, top: usize, doc: usize) {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            self.set_owner(n, doc);
            self.push_owned(n, &mut stack);
        }
    }

    /// A new, empty document with a handle, so it lasts as long as the slab.
    pub(crate) fn new_document(&mut self, info: DocInfo) -> Result<usize, DomError> {
        self.check_room()?;
        let name = QualName::new(None, ns!(), LocalName::from(DOCUMENT_NAME));
        self.recharge(0, name_cost(&name))?;
        let id = self.inner.mutate().create_element(name, Vec::new());
        self.owners.insert(id, id);
        self.docs.insert(id, info);
        self.handle_for(id)?;
        Ok(id)
    }

    /// A document as `DOMImplementation` and `new Document()` make them.
    /// `html`, or XML of `content_type`.
    pub(crate) fn blank_info(&self, html: bool, content_type: &'static str) -> DocInfo {
        DocInfo {
            url_charged: false,
            frame_element: None,
            html,
            xml_interface: false,
            quirks: false,
            content_type,
            frame: false,
            url: Arc::from("about:blank"),
        }
    }

    /// The `contentType` of `createDocument` for a document element in
    /// namespace `ns_uri` (DOM §4.5.1).
    pub(crate) fn xml_content_type(ns_uri: Option<&str>) -> &'static str {
        match ns_uri {
            Some("http://www.w3.org/1999/xhtml") => APPLICATION_XHTML,
            Some("http://www.w3.org/2000/svg") => "image/svg+xml",
            _ => APPLICATION_XML,
        }
    }

    /// Runs `f` with `doc` as the document new nodes join.
    pub(crate) fn making_in<R>(&mut self, doc: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        let old = std::mem::replace(&mut self.making, doc);
        let r = f(self);
        self.making = old;
        r
    }

    /// Document `doc`'s URL. See [`Document::set_url`].
    pub(crate) fn url(&self, doc: usize) -> &str {
        &self.info(doc).url
    }

    /// Sets the URL of the page's document (`document.URL`), which the
    /// documents it makes through `DOMParser` take on. `about:blank` until set.
    /// An XML page's `contentType` then goes by the URL's extension, as a
    /// frame's does, for want of a Content-Type. `pub` for the runner and the
    /// facade; the crate is unpublished.
    pub fn set_url(&mut self, url: &str) {
        let root = self.root();
        if let Some(d) = self.docs.get_mut(&root) {
            d.url = Arc::from(url);
            if !d.html {
                d.content_type = frame_content_type(url).unwrap_or(APPLICATION_XHTML);
            }
        }
    }

    /// A script's change of `doc`'s URL (`pushState`, a fragment). Rust holds
    /// these outside the JS heap, and every document `DOMParser` makes keeps
    /// the URL it was made under, so each is charged to `max_text_bytes` until
    /// nothing holds it.
    pub(crate) fn write_url(&mut self, doc: usize, url: &str) -> Result<(), DomError> {
        let Some(d) = self.docs.get(&doc) else {
            return Ok(());
        };
        if d.url.as_ref() == url {
            return Ok(());
        }
        let credit = if d.url_charged && Arc::strong_count(&d.url) == 1 {
            d.url.len()
        } else {
            0
        };
        self.recharge(credit, url.len())?;
        self.observer_bytes = (self.observer_bytes + url.len()).saturating_sub(credit);
        if let Some(d) = self.docs.get_mut(&doc) {
            d.url = Arc::from(url);
            d.url_charged = true;
        }
        Ok(())
    }

    /// Note a node of a kind blitz-dom doesn't have.
    pub(crate) fn special(&self, id: usize) -> Option<&Special> {
        if self.specials.is_empty() {
            return None;
        }
        self.specials.get(&id)
    }

    pub(crate) fn is_cdata(&self, id: usize) -> bool {
        matches!(self.special(id), Some(Special::Cdata))
    }

    /// Whether `id` is a Text node and no CDATA section: DOM "exclusive Text
    /// node", what `normalize` and `wholeText` go by.
    pub(crate) fn is_exclusive_text(&self, id: usize) -> bool {
        self.kind(id) == Some(Kind::Text) && !self.is_cdata(id)
    }

    /// A new `DocumentType`.
    pub(crate) fn create_doctype(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> Result<usize, DomError> {
        self.check_room()?;
        let special = Special::Doctype {
            name: name.to_owned(),
            public_id: public_id.to_owned(),
            system_id: system_id.to_owned(),
        };
        self.recharge(0, special.bytes())?;
        let id = self.inner.mutate().create_comment_node();
        self.specials.insert(id, special);
        self.born(id);
        Ok(id)
    }

    /// A new `ProcessingInstruction`.
    pub(crate) fn create_instruction(
        &mut self,
        target: &str,
        data: &str,
    ) -> Result<usize, DomError> {
        self.check_room()?;
        let special = Special::Instruction {
            target: target.to_owned(),
        };
        self.recharge(0, special.bytes().saturating_add(data.len()))?;
        let id = self.inner.mutate().create_comment_node();
        self.side_data.insert(id, data.to_owned());
        self.specials.insert(id, special);
        self.born(id);
        Ok(id)
    }

    /// A new `CDATASection`.
    pub(crate) fn create_cdata(&mut self, data: &str) -> Result<usize, DomError> {
        let id = self.create_text(data)?;
        self.specials.insert(id, Special::Cdata);
        Ok(id)
    }

    /// A node just made joins the document being made in.
    fn born(&mut self, id: usize) {
        if self.making != self.root() {
            self.owners.insert(id, self.making);
        }
    }

    /// DOMParser's `text/html` (HTML §8.5.3): a new document parsed from
    /// `html`, with scripting off, its URL the page's. Fits the limits with
    /// everything else in the render. `Ok(None)` if the time limit passed.
    pub(crate) fn parse_html_document(
        &mut self,
        html: &str,
        url: Arc<str>,
    ) -> Result<Option<usize>, DomError> {
        let mut info = self.blank_info(true, TEXT_HTML);
        info.url = url;
        let doc = self.new_document(info)?;
        let room = self.limits.max_nodes.saturating_sub(self.node_count());
        let deadline = self.deadline.clone();
        let parsed = parse::parse_document(
            &mut self.inner.mutate(),
            doc,
            html,
            ParseLimits {
                max_nodes: room,
                max_depth: self.limits.max_depth,
                max_attributes: self.limits.max_attributes,
            },
            false,
            || deadline.as_ref().is_some_and(Deadline::passed),
        );
        self.side_data.extend(parsed.strings);
        self.specials.extend(parsed.specials);
        self.templates.extend(parsed.templates);
        if let Some(d) = self.docs.get_mut(&doc) {
            d.quirks = parsed.quirks;
        }
        // Charged here and credited by the drops below if it is refused.
        let bytes = self.bytes_under(doc) - self.node_bytes(doc);
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
        let timed_out = deadline.as_ref().is_some_and(Deadline::passed);
        let over = parsed.over_limit && !timed_out && !parsed.over_attributes;
        let limit = if self.node_count() > self.limits.max_nodes || over {
            let deep = self.height(doc) - 1 > self.limits.max_depth;
            Some(if deep {
                DomError::TreeTooDeep {
                    limit: self.limits.max_depth,
                }
            } else {
                DomError::NodeLimit {
                    limit: self.limits.max_nodes,
                }
            })
        } else if self.held_bytes > self.limits.max_text_bytes {
            Some(DomError::TextLimit {
                limit: self.limits.max_text_bytes,
            })
        } else if self.height(doc) - 1 > self.limits.max_depth {
            Some(DomError::TreeTooDeep {
                limit: self.limits.max_depth,
            })
        } else {
            None
        };
        if parsed.over_style || parsed.over_attributes || limit.is_some() || timed_out {
            self.remove_children_quietly(doc);
            if parsed.over_style {
                return Err(style_attr::limit_error());
            }
            if parsed.over_attributes {
                return Err(DomError::AttributeLimit {
                    limit: self.limits.max_attributes,
                });
            }
            return match limit {
                Some(e) => Err(e),
                None => Ok(None),
            };
        }
        self.own_subtree(doc, doc);
        Ok(Some(doc))
    }

    /// The most attributes an element may have.
    pub(crate) fn max_attributes(&self) -> usize {
        self.limits.max_attributes
    }

    /// The most bytes of strings a document may hold.
    pub(crate) fn max_text_bytes(&self) -> usize {
        self.limits.max_text_bytes
    }

    /// The deepest nesting allowed.
    pub(crate) fn max_depth(&self) -> usize {
        self.limits.max_depth
    }

    /// DOMParser for an XML type (HTML §8.5.3), or a frame's XML document: a
    /// new `XMLDocument` of `content_type`. Input that isn't well-formed gives
    /// a document holding only a `parsererror` element, as the standard says;
    /// a limit is an error. `Ok(None)` if the time limit passed.
    pub(crate) fn parse_xml_document(
        &mut self,
        text: &str,
        content_type: &'static str,
        url: Arc<str>,
    ) -> Result<Option<usize>, DomError> {
        let mut info = self.blank_info(false, content_type);
        info.url = url;
        info.xml_interface = true;
        let doc = self.new_document(info)?;
        self.fill_xml_document(doc, text)
    }

    /// `<iframe>` `id`'s document (HTML §4.8.5): none unless it is connected;
    /// its initial `about:blank` until one has loaded.
    pub(crate) fn frame_document(&mut self, id: usize) -> Result<Option<usize>, DomError> {
        let owner = self.owner_of_node(id);
        if !self.is_connected(id) || (owner != self.root() && !self.info(owner).frame) {
            // Removal discards the browsing context, and a document with none
            // of its own can't give one.
            self.frames.remove(&id);
            self.frames_started.remove(&id);
            return Ok(None);
        }
        if let Some(&d) = self.frames.get(&id) {
            return Ok(Some(d));
        }
        let Some(d) = self.parse_html_document("", Arc::from("about:blank"))? else {
            return Ok(None);
        };
        if let Some(info) = self.docs.get_mut(&d) {
            info.frame = true;
            info.frame_element = Some(id);
        }
        self.frames.insert(id, d);
        Ok(Some(d))
    }

    /// `<iframe>`s in the page and its frames that haven't started to load,
    /// in tree order: (element, `srcdoc`, `src`). At most `room` of them, so
    /// a page's frames are bounded whatever it builds.
    pub(crate) fn frames_to_load(&mut self) -> Vec<(usize, Option<String>, Option<String>)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            stack.extend(self.children(n).iter().rev().copied());
            if !self.is_html_element(n, &["iframe"]) {
                continue;
            }
            if let Some(&d) = self.frames.get(&n) {
                stack.push(d);
            }
            if !self.frames_started.contains(&n) && self.frame_count + out.len() < MAX_FRAMES {
                let src = self.attribute(n, "src").filter(|s| !s.is_empty());
                out.push((n, self.attribute(n, "srcdoc"), src));
            }
        }
        self.frame_count += out.len();
        for (n, ..) in &out {
            self.frames_started.insert(*n);
        }
        out
    }

    /// Whether frame document `doc` is fully active: its `<iframe>` is
    /// connected and still has it as its document, in the page or in a frame
    /// that is. Linear in the chain, which `max_nodes` bounds, and each
    /// frame's document can only be one element's.
    pub(crate) fn frame_is_attached(&self, doc: usize) -> bool {
        let mut at = doc;
        for _ in 0..=self.frames.len() {
            if at == self.root() {
                return true;
            }
            let Some(e) = self.info(at).frame_element else {
                return false;
            };
            if self.frames.get(&e) != Some(&at) || !self.is_connected(e) {
                return false;
            }
            at = self.owner_of_node(e);
        }
        false
    }

    /// HTML §4.8.5: which attribute changes "process the iframe attributes":
    /// `srcdoc` set, changed or removed, and `src` likewise with no `srcdoc`.
    fn note_frame_attribute(&mut self, id: usize, name: &QualName) {
        if self.frames_started.is_empty() || !name.ns.is_empty() {
            return;
        }
        let wanted = match name.local.as_ref() {
            "srcdoc" => true,
            "src" => self.attribute(id, "srcdoc").is_none(),
            _ => false,
        };
        if wanted {
            self.frames_started.remove(&id);
        }
    }

    /// Keeps a frame's source until a call parses it, under that call's limits.
    pub(crate) fn queue_frame(&mut self, id: usize, text: Option<String>, url: String) {
        self.frame_sources.push_back((id, text, url));
    }

    /// Parses the next queued frame, inside a native call, so the time limit,
    /// a cancel and the panic guard all reach the parse. Gives `<iframe>` its
    /// document and queues its `load`. `text` `None` keeps what it has (a
    /// failed load still fires `load`, as a 404 does). The type goes by the
    /// URL's extension: there is no Content-Type here. A frame's scripts don't
    /// run. `Ok(None)` if nothing is queued; `Ok(Some(false))` if the time
    /// limit passed.
    pub(crate) fn load_next_frame(&mut self) -> Result<Option<bool>, DomError> {
        let Some((id, text, url)) = self.frame_sources.pop_front() else {
            return Ok(None);
        };
        if !self.is_connected(id) {
            return Ok(Some(true));
        }
        let url: Arc<str> = Arc::from(url);
        let parsed = match text {
            None => self.frame_document(id)?,
            Some(text) => {
                let parsed = match frame_content_type(&url) {
                    Some(t) => self.parse_xml_document(&text, t, url)?,
                    None => self.parse_html_document(&text, url)?,
                };
                if parsed.is_none() {
                    return Ok(Some(false));
                }
                parsed
            }
        };
        if let Some(d) = parsed {
            if let Some(info) = self.docs.get_mut(&d) {
                info.frame = true;
                info.frame_element = Some(id);
            }
            self.frames.insert(id, d);
        }
        let h = self.handle_for(id)?;
        self.frame_loads.push_back(h);
        Ok(Some(true))
    }

    /// The next `<iframe>` whose `load` is due, 0 for none.
    pub(crate) fn next_frame(&mut self) -> Handle {
        self.frame_loads.pop_front().unwrap_or(0)
    }

    /// HTML §2.4.1 "document base URL": the URL, or the first `<base href>`
    /// in the head parsed against it. A `<base>` outside the head is ignored
    /// here (a gap): reflected URLs read this often, and a walk of the head
    /// stays small.
    pub(crate) fn base_url(&self, doc: usize) -> String {
        let url = self.url(doc).to_owned();
        let Some(head) = self.head(doc) else {
            return url;
        };
        let base = self.children(head).iter().find_map(|&c| {
            if self.is_html_element(c, &["base"]) {
                self.attribute(c, "href")
            } else {
                None
            }
        });
        let parsed = base.and_then(|b| url::Url::parse(&url).and_then(|u| u.join(&b)).ok());
        parsed.map_or(url, String::from)
    }

    /// HTML §7.4.6.3 "find a potential indicated element", for the document
    /// being queried: an element with the URL's fragment, as written and then
    /// percent-decoded, as its `id`, or an `<a>` with it as its `name`.
    pub(crate) fn target_element(&self) -> Option<usize> {
        let doc = self.making();
        let url = url::Url::parse(self.url(doc)).ok()?;
        let fragment = url.fragment().filter(|f| !f.is_empty())?;
        let decoded = percent_decode(fragment);
        let find = |wanted: &str| {
            self.element_by_id(doc, wanted).or_else(|| {
                let mut stack = vec![doc];
                while let Some(n) = stack.pop() {
                    if self.is_html_element(n, &["a"])
                        && self.attribute(n, "name").as_deref() == Some(wanted)
                    {
                        return Some(n);
                    }
                    stack.extend(self.children(n).iter().rev());
                }
                None
            })
        };
        find(fragment).or_else(|| find(&decoded))
    }

    /// `src`, resolved against the page's URL: what a frame's document is
    /// called. The resolver is asked for `src` as the page spelled it, without
    /// the fragment.
    pub(crate) fn frame_url(&self, src: &str) -> String {
        let base = url::Url::parse(&self.info(self.root()).url);
        base.and_then(|b| b.join(src))
            .map_or_else(|_| src.to_owned(), String::from)
    }

    /// Parses `text` under the empty document `doc`.
    fn fill_xml_document(&mut self, doc: usize, text: &str) -> Result<Option<usize>, DomError> {
        self.fill_xml_until(doc, text, &|| false)
    }

    /// [`fill_xml_document`](Self::fill_xml_document) under a stop check.
    fn fill_xml_until(
        &mut self,
        doc: usize,
        text: &str,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<usize>, DomError> {
        let seed = HashMap::new();
        let done = self.making_in(doc, |d| {
            xml::parse(d, doc, text, xml::Mode::Document, &seed, 0, stop)
        });
        match done {
            Ok(()) => Ok(Some(doc)),
            Err(xml::Stop::Halted) => {
                self.remove_children_quietly(doc);
                Err(DomError::Stopped)
            }
            Err(xml::Stop::Limit(_)) => {
                self.remove_children_quietly(doc);
                match self.fault.take() {
                    Some(e) => Err(e),
                    None => Ok(None),
                }
            }
            Err(xml::Stop::Malformed(message)) => {
                self.remove_children_quietly(doc);
                self.making_in(doc, |d| {
                    let e = d.create_element_with(xml::error_name(), Vec::new())?;
                    d.place_quietly(doc, &[e], None);
                    let t = d.create_text(&message)?;
                    d.place_quietly(e, &[t], None);
                    Ok(Some(doc))
                })
            }
        }
    }

    /// Parses `text` as XML element content into a new fragment, in the
    /// document of `context`, with the namespaces in scope there (DOM Parsing
    /// §2.1, "XML fragment parsing"). `Err` with the error for a `SyntaxError`
    /// or a fatal stop.
    pub(crate) fn parse_xml_fragment(
        &mut self,
        context: usize,
        parent: usize,
        text: &str,
    ) -> Result<usize, xml::Stop> {
        // Innermost first: a prefix seen stays as it is.
        let mut scope: HashMap<String, Namespace> = HashMap::new();
        let mut at = Some(context);
        while let Some(n) = at {
            for a in self.node(n).attrs().into_iter().flatten() {
                self.poll_by(1 + a.value.len() / 64)?;
                if a.name.ns.as_ref() == "http://www.w3.org/2000/xmlns/" {
                    let prefix = match &a.name.prefix {
                        Some(_) => a.name.local.to_string(),
                        None => String::new(),
                    };
                    scope
                        .entry(prefix)
                        .or_insert_with(|| Namespace::from(a.value.as_str()));
                }
            }
            if let Some(q) = self.element_name(n)
                && !q.ns.is_empty()
            {
                let prefix = q.prefix.as_deref().unwrap_or("").to_owned();
                scope.entry(prefix).or_insert_with(|| q.ns.clone());
            }
            at = self.parent(n);
        }
        let frag = self
            .create_fragment()
            .map_err(|e| xml::Stop::Limit(self.fail(e)))?;
        let depth = self.depth(parent);
        match xml::parse(
            self,
            frag,
            text,
            xml::Mode::Fragment,
            &scope,
            depth,
            &|| false,
        ) {
            Ok(()) => Ok(frag),
            Err(e) => {
                self.drop_subtree(frag);
                Err(e)
            }
        }
    }

    /// `id`'s children, serialized (HTML §13.3, `innerHTML`).
    pub(crate) fn inner_html(&self, id: usize) -> String {
        let mut out = String::new();
        let top = if self.is_html_template(id) {
            self.template_of(id)
        } else {
            Some(id)
        };
        for &c in top.iter().flat_map(|&t| self.children(t)) {
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

    /// [`to_html`](Self::to_html) for output a caller has to bound: at most
    /// `max_bytes` (a little more may be built, never kept) or
    /// [`DomError::OutputLimit`], and `stop` is asked as it goes, however long
    /// a text is, ending in [`DomError::Stopped`].
    pub fn to_html_bounded(
        &self,
        max_bytes: usize,
        stop: impl Fn() -> bool,
    ) -> Result<String, DomError> {
        let mut out = String::new();
        if let Some(id) = self.document_element() {
            let mut steps = 0u32;
            self.serialize_with(id, &mut out, &mut |out| {
                if out.len() > max_bytes {
                    return Err(DomError::OutputLimit { limit: max_bytes });
                }
                steps = steps.wrapping_add(1);
                if steps % STOP_EVERY == 0 && stop() {
                    return Err(DomError::Stopped);
                }
                Ok(())
            })?;
        }
        if out.len() > max_bytes {
            return Err(DomError::OutputLimit { limit: max_bytes });
        }
        Ok(out)
    }

    fn serialize(&self, top: usize, out: &mut String) {
        // The check never fails, so neither does this.
        let _ = self.serialize_with(top, out, &mut |_| Ok(()));
    }

    /// Serializes `top`, calling `check` at each node and every
    /// [`TEXT_SLICE`] bytes of a string, so a check sees the output grow.
    fn serialize_with(
        &self,
        top: usize,
        out: &mut String,
        check: &mut dyn FnMut(&str) -> Result<(), DomError>,
    ) -> Result<(), DomError> {
        enum Step {
            Open(usize),
            Close(usize),
        }
        let mut stack = vec![Step::Open(top)];
        while let Some(step) = stack.pop() {
            check(out)?;
            match step {
                Step::Close(id) => {
                    if let Some(e) = self.node(id).element_data() {
                        out.push_str("</");
                        push_sliced(&e.name.local, true, false, out, check)?;
                        out.push('>');
                    }
                }
                Step::Open(id) => {
                    let node = self.node(id);
                    if matches!(self.kind(id), Some(Kind::Document | Kind::DocumentFragment)) {
                        stack.extend(node.children.iter().rev().map(|&k| Step::Open(k)));
                        continue;
                    }
                    match &node.data {
                        NodeData::Text(_) => {
                            // §13.3: text is written unescaped when its
                            // parent is an HTML style, script, xmp, iframe,
                            // noembed, noframes or plaintext element (an SVG
                            // <style> or <script> is escaped), or a noscript
                            // element when scripting is enabled for the
                            // node, which is in the page's document only.
                            let raw = node
                                .parent
                                .and_then(|p| self.node(p).element_data())
                                .is_some_and(|e| {
                                    e.name.ns == ns!(html)
                                        && match e.name.local.as_ref() {
                                            "style" | "script" | "xmp" | "iframe" | "noembed"
                                            | "noframes" | "plaintext" => true,
                                            "noscript" => self.owner_of_node(id) == self.root(),
                                            _ => false,
                                        }
                                });
                            push_sliced(self.char_data(id), raw, false, out, check)?;
                        }
                        // §13.3: comment data goes out unescaped.
                        NodeData::Comment if self.special(id).is_some() => {
                            match self.special(id) {
                                // §13.3: "<!DOCTYPE " and the name.
                                Some(Special::Doctype { name, .. }) => {
                                    out.push_str("<!DOCTYPE ");
                                    out.push_str(name);
                                    out.push('>');
                                }
                                // §13.3: "<?", target, a space, data, ">".
                                Some(Special::Instruction { target }) => {
                                    out.push_str("<?");
                                    out.push_str(target);
                                    out.push(' ');
                                    out.push_str(self.char_data(id));
                                    out.push('>');
                                }
                                _ => {}
                            }
                        }
                        NodeData::Comment => {
                            out.push_str("<!--");
                            push_sliced(self.char_data(id), true, false, out, check)?;
                            out.push_str("-->");
                        }
                        NodeData::Document => {}
                        NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                            out.push('<');
                            push_sliced(&e.name.local, true, false, out, check)?;
                            for a in e.attrs() {
                                out.push(' ');
                                if let Some(prefix) = attribute_prefix(&a.name) {
                                    push_sliced(prefix, true, false, out, check)?;
                                    out.push(':');
                                }
                                push_sliced(&a.name.local, true, false, out, check)?;
                                out.push_str("=\"");
                                push_sliced(&a.value, false, true, out, check)?;
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
                                // children, of which a script can give it
                                // some; one a script made may have no
                                // content yet.
                                let kids: &[usize] = if self.is_html_template(id) {
                                    self.template_of(id).map_or(&[], |c| &self.node(c).children)
                                } else {
                                    &node.children
                                };
                                stack.extend(kids.iter().rev().map(|&k| Step::Open(k)));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
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

    pub(crate) fn new_handle(&mut self, object: Object) -> Result<Handle, DomError> {
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
            Owned::List(ListKind::ByTagNs(n, l)) => {
                (n.as_ref().map_or(0, String::len) + l.len()).saturating_mul(2)
            }
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
                    let html = e.ns == ns!(html) && self.in_html_document(n);
                    let wanted = if html { &lower } else { name };
                    Ok(name == "*" || qualified_is(e, wanted))
                })?
            }
            // DOM §4.2.6.2 "list of elements with namespace and local name".
            ListKind::ByTagNs(namespace, local) => self.descendants(owner, |n| {
                let Some(e) = self.element_name(n) else {
                    return Ok(false);
                };
                let ns_ok = namespace.as_ref().is_none_or(|w| w == e.ns.as_ref());
                Ok(ns_ok && (local == "*" || local == e.local.as_ref()))
            })?,
            ListKind::ByClass(names) => self.elements_with_classes(owner, names)?,
        })
    }

    /// `getElementsByClassName`: linear in the names and in the classes
    /// read, however many of each there are. Only looked up, so hash order
    /// can't reach any output.
    fn elements_with_classes(&self, owner: usize, names: &str) -> BindingResult<Vec<usize>> {
        // Case-insensitive in quirks mode (DOM §4.2.6.2).
        let folded = |s| {
            if self.is_quirks() {
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
            NodeData::Element(e) if e.name.local.as_ref() == DOCUMENT_NAME => Some(Kind::Document),
            NodeData::Element(_) => Some(Kind::Element),
            NodeData::Text(_) => Some(Kind::Text),
            NodeData::Comment => Some(match self.special(id) {
                Some(Special::Doctype { .. }) => Kind::DocumentType,
                Some(Special::Instruction { .. }) => Kind::ProcessingInstruction,
                _ => Kind::Comment,
            }),
            // Layout-only boxes; never reachable from the tree links scripts
            // follow before layout.
            NodeData::AnonymousBlock(_) => None,
        }
    }

    /// The document node's id.
    pub(crate) fn root(&self) -> usize {
        self.inner.root_node().id
    }

    /// The page's document element.
    pub(crate) fn document_element(&self) -> Option<usize> {
        self.document_element_of(self.root())
    }

    /// `doc`'s document element: its element child, of which it has one.
    pub(crate) fn document_element_of(&self, doc: usize) -> Option<usize> {
        let mut children = self.children(doc).iter().copied();
        children.find(|&c| self.kind(c) == Some(Kind::Element))
    }

    /// `doc`'s doctype child.
    pub(crate) fn doctype_of(&self, doc: usize) -> Option<usize> {
        let mut children = self.children(doc).iter().copied();
        children.find(|&c| self.kind(c) == Some(Kind::DocumentType))
    }

    /// Whether `id` is an HTML element with one of these local names.
    fn is_html_element(&self, id: usize, locals: &[&str]) -> bool {
        self.node(id)
            .element_data()
            .is_some_and(|e| e.name.ns == ns!(html) && locals.contains(&e.name.local.as_ref()))
    }

    /// The first child of the html element (HTML "the html element": the
    /// document element, if it is one) with one of these local names.
    fn html_child(&self, doc: usize, locals: &[&str]) -> Option<usize> {
        let html = self
            .document_element_of(doc)
            .filter(|&e| self.is_html_element(e, &["html"]))?;
        let mut children = self.node(html).children.iter().copied();
        children.find(|&c| self.is_html_element(c, locals))
    }

    pub(crate) fn head(&self, doc: usize) -> Option<usize> {
        self.html_child(doc, &["head"])
    }

    /// HTML "the body element": the first body or frameset child.
    pub(crate) fn body(&self, doc: usize) -> Option<usize> {
        self.html_child(doc, &["body", "frameset"])
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
    fn title_element(&self, doc: usize) -> BindingResult<Option<usize>> {
        let Some(top) = self.document_element_of(doc) else {
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
    pub(crate) fn title(&self, doc: usize) -> BindingResult<String> {
        Ok(self
            .title_element(doc)?
            .map(|t| self.child_text(t))
            .map(|t| t.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default())
    }

    /// `document.title` set: the title's content, a new `<title>` in the
    /// head if there is none; nothing without a head.
    pub(crate) fn set_title(&mut self, doc: usize, value: &str) -> BindingResult<()> {
        let title = match self.title_element(doc)? {
            Some(t) => t,
            None => {
                let Some(head) = self.head(doc) else {
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

    /// Whether the running member's document is in quirks mode.
    pub(crate) fn is_quirks(&self) -> bool {
        self.info(self.making).quirks
    }

    pub(crate) fn quirks_mode(&self) -> style::context::QuirksMode {
        if self.is_quirks() {
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

    /// Whether `id`'s tree is a document's.
    pub(crate) fn is_connected(&self, id: usize) -> bool {
        let mut at = id;
        while let Some(p) = self.node(at).parent {
            at = p;
        }
        self.kind(at) == Some(Kind::Document)
    }

    /// The node name for scripts: an HTML element's qualified name in upper
    /// case (DOM §4.9 `tagName`), any other element's as it is.
    pub(crate) fn node_name(&self, id: usize) -> String {
        match &self.node(id).data {
            NodeData::Document => "#document".to_owned(),
            NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                let mut name = qualified_name(&e.name);
                if e.name.ns == ns!(html) && self.in_html_document(id) {
                    name.make_ascii_uppercase();
                }
                name
            }
            NodeData::Text(_) if self.is_cdata(id) => "#cdata-section".to_owned(),
            NodeData::Text(_) => "#text".to_owned(),
            NodeData::Comment => match self.special(id) {
                Some(Special::Doctype { name, .. }) => name.clone(),
                Some(Special::Instruction { target }) => target.clone(),
                _ => "#comment".to_owned(),
            },
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
        let name = if e.name.ns == ns!(html) && self.in_html_document(id) {
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
                let html = self.element_name(id).is_some_and(|n| n.ns == ns!(html))
                    && self.in_html_document(id);
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
        if name.local.as_ref().starts_with("on")
            && self.element_name(id).is_some_and(|n| {
                n.ns == ns!(html) && matches!(n.local.as_ref(), "body" | "frameset")
            })
        {
            self.handler_source = Some(self.handle_for(id)?);
        }
        // Any namespace: blitz-dom goes by the local name.
        if name.local.as_ref() == "style" && !style_attr::fits(value) {
            return Err(style_attr::limit_error());
        }
        let credit = self.attribute_bytes(id, &name);
        if credit == 0 && self.attribute_count(id) >= self.limits.max_attributes {
            return Err(DomError::AttributeLimit {
                limit: self.limits.max_attributes,
            });
        }
        let charge = attribute_cost(&name, value);
        self.recharge(credit, charge)?;
        self.note_attribute(id, &name);
        self.version += 1;
        // Another load, in the next round, under the render's `MAX_FRAMES`.
        self.note_frame_attribute(id, &name);
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

    /// A new `Attr` with no element, in document `doc`: an `Object::Attr`
    /// whose owner is the document and which keeps its own value.
    pub(crate) fn free_attr(&mut self, doc: usize, name: QualName) -> Result<Handle, DomError> {
        let bytes = name_cost(&name);
        self.recharge(0, bytes)?;
        self.retained_bytes += bytes;
        self.new_handle(Object::Attr(doc, name, Some(String::new())))
    }

    /// Puts the free `Attr` `handle` on `element` under `name`, with `value`,
    /// which it held: DOM §4.9 "replace an attribute" if there is one, `old`,
    /// whose object keeps the old value, and this one has its place.
    pub(crate) fn attach_attr(
        &mut self,
        handle: Handle,
        element: usize,
        name: QualName,
        value: String,
        old: Option<QualName>,
    ) -> Result<(), DomError> {
        let target = old.clone().unwrap_or_else(|| name.clone());
        if old.is_some() {
            self.keep_attr_value(element, &target);
        }
        self.write_attribute(element, target.clone(), &value)?;
        if target != name {
            // Same namespace and local name, so only the prefix differs: blitz-dom
            // has seen what it goes by, and the entry keeps its place.
            self.recharge(name_cost(&target), name_cost(&name))?;
            if let Some(e) = self.element_mut(element)
                && let Some(a) = e.attrs.iter_mut().find(|a| a.name == target)
            {
                a.name = name.clone();
            }
        }
        self.credit(value.len());
        self.retained_bytes -= value.len();
        self.owned.insert((element, Owned::Attr(name)), handle);
        if let Some(Object::Attr(e, _, kept)) = self.object_mut(handle) {
            *e = element;
            *kept = None;
        }
        Ok(())
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
            NodeData::Text(_) | NodeData::Comment => self
                .char_data(id)
                .len()
                .saturating_add(self.special(id).map_or(0, Special::bytes)),
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
        self.note_frame_attribute(id, &name);
        self.keep_attr_value(id, &name);
        self.note_attribute(id, &name);
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
        if matches!(self.kind(id), Some(Kind::Document | Kind::DocumentType)) {
            return None;
        }
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
            Some(Kind::Text | Kind::Comment | Kind::ProcessingInstruction) => {
                // §4.11 "replace data", of all of it.
                let count = self.has_range_point(id).then(|| self.node_length(id));
                self.write_char_data(id, text)?;
                if let Some(count) = count {
                    let length = u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX);
                    self.ranges_replaced_data(id, 0, count, length);
                }
                Ok(())
            }
            Some(Kind::Element | Kind::DocumentFragment) => {
                // Made first: a refused write leaves the children alone.
                let t = if text.is_empty() {
                    None
                } else {
                    Some(self.create_text(text)?)
                };
                self.replace_all(id, t);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Writes a character data node's data; what happens to ranges is the
    /// caller's.
    pub(crate) fn write_char_data(&mut self, id: usize, text: &str) -> Result<(), DomError> {
        match self.kind(id) {
            Some(Kind::Text) => self.set_text_data(id, text),
            _ => self.set_side_data(id, text),
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

    /// Removes every child of `id`, with no mutation record. A child whose
    /// subtree holds a handle stays alive, detached; any other is dropped.
    ///
    /// Iterative, and linear in the subtree: blitz-dom's own subtree drop
    /// recurses per level, and removing children one at a time scans the
    /// parent's list each time.
    pub(crate) fn remove_children_quietly(&mut self, id: usize) {
        let gone = self.children(id).to_vec();
        self.replace_child_list(id, Vec::new(), gone);
    }

    /// Makes `kept` the children of `id` and removes `gone`, its other
    /// children, as [`remove_children_quietly`](Self::remove_children_quietly)
    /// does: one pass, however wide the list.
    pub(crate) fn replace_child_list(&mut self, id: usize, kept: Vec<usize>, gone: Vec<usize>) {
        self.ranges_removing(id, &gone);
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
            self.credit(self.node_bytes(n));
            self.side_data.remove(&n);
            self.specials.remove(&n);
            self.owners.remove(&n);
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
        let id = self.inner.mutate().create_text_node(text);
        self.born(id);
        Ok(id)
    }

    pub(crate) fn create_comment(&mut self, data: &str) -> Result<usize, DomError> {
        self.check_room()?;
        self.recharge(0, data.len())?;
        let id = self.inner.mutate().create_comment_node();
        self.side_data.insert(id, data.to_owned());
        self.born(id);
        Ok(id)
    }

    pub(crate) fn create_fragment(&mut self) -> Result<usize, DomError> {
        self.check_room()?;
        let name = QualName::new(None, ns!(), fragment_name());
        self.recharge(0, name_cost(&name))?;
        let id = self.inner.mutate().create_element(name, Vec::new());
        self.born(id);
        Ok(id)
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
        self.born(id);
        Ok(id)
    }

    /// `createElement` (DOM §4.5): in the document being made in, whose type
    /// decides the namespace and whether the name is lower-cased; `None` for a
    /// name the DOM rejects with InvalidCharacterError.
    pub(crate) fn create_element(&mut self, name: &str) -> Result<Option<usize>, DomError> {
        if !is_valid_element_name(name) {
            return Ok(None);
        }
        self.check_room()?;
        let info = self.info(self.making);
        let (html_doc, xhtml) = (info.html, info.content_type == APPLICATION_XHTML);
        let local = if html_doc {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        let namespace = if html_doc || xhtml { ns!(html) } else { ns!() };
        // Before the atom exists: it holds the whole name, in Rust.
        self.recharge(0, local.len() + namespace.len())?;
        let qual = QualName::new(None, namespace, LocalName::from(local));
        let id = self.inner.mutate().create_element(qual, Vec::new());
        self.born(id);
        Ok(Some(id))
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
            self.detach_quietly(id);
        }
    }

    /// [`detach`](Self::detach) with the "suppress observers" flag of DOM
    /// §4.2.3 "remove".
    pub(crate) fn detach_quietly(&mut self, id: usize) {
        if let Some(parent) = self.node(id).parent {
            self.ranges_removing(parent, &[id]);
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
        if self.is_observed() && !nodes.is_empty() {
            let previous = match before {
                Some(b) => self.sibling(b, false),
                None => self.children(parent).last().copied(),
            };
            self.note_child_list(parent, nodes, &[], previous, before);
        }
        self.place_quietly(parent, nodes, before);
    }

    /// [`place`](Self::place) with the "suppress observers" flag of DOM
    /// §4.2.3 "insert".
    pub(crate) fn place_quietly(&mut self, parent: usize, nodes: &[usize], before: Option<usize>) {
        self.ranges_inserting(parent, before, nodes.len());
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

    pub(crate) fn is_observed(&self) -> bool {
        !self.registrations.is_empty()
    }

    /// DOM §4.3 "queue a mutation record": `wanted` picks the registrations
    /// on `target` and its ancestors that are interested; each observer once,
    /// with the old value if any of its registrations asked; `record` is told
    /// whether to copy it.
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
            let Some(r) = record(self, old) else {
                return;
            };
            match self.new_handle(Object::Record(Box::new(r))) {
                Ok(h) => {
                    if let Some(q) = self.record_queues.get_mut(observer as usize) {
                        q.push_back(h);
                        self.queued_records += 1;
                    }
                }
                Err(e) => {
                    self.fault.get_or_insert(e);
                    return;
                }
            }
        }
    }

    /// Charges `bytes` of a record's strings to the document, for its life.
    /// `None` and the call's fault if they don't fit.
    fn charge_record(&mut self, bytes: usize) -> Option<()> {
        match self.recharge(0, bytes) {
            Ok(()) => {
                self.observer_bytes += bytes;
                Some(())
            }
            Err(e) => {
                self.fault.get_or_insert(e);
                None
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
        if let Some(n) = id
            && let Err(e) = self.handle_for(n)
        {
            self.fault.get_or_insert(e);
        }
        id
    }

    pub(crate) fn note_child_list(
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
            |doc, old| {
                let namespace = Some(name.ns.to_string()).filter(|n| !n.is_empty());
                let old_value = if old {
                    doc.attribute_value(id, name)
                } else {
                    None
                };
                let strings = local.len()
                    + namespace.as_ref().map_or(0, String::len)
                    + old_value.as_ref().map_or(0, String::len);
                doc.charge_record(strings)?;
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
                    namespace,
                    old_value,
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
            |doc, old| {
                let old_value = old.then(|| doc.char_data(id).to_owned());
                doc.charge_record(old_value.as_ref().map_or(0, String::len))?;
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
                    old_value,
                })
            },
        );
    }

    pub(crate) fn new_observer(&mut self) -> u32 {
        self.record_queues.push(VecDeque::new());
        u32::try_from(self.record_queues.len() - 1).unwrap_or(u32::MAX)
    }

    /// `observe`: a second call on one node replaces the first (§4.3.1).
    /// Charged to [`DomLimits::max_text_bytes`]: filters are copies of
    /// strings of any size, and registrations grow as observers × nodes.
    pub(crate) fn add_registration(
        &mut self,
        observer: u32,
        target: usize,
        flags: u32,
        names: &str,
    ) -> Result<(), DomError> {
        let credit = self
            .registrations
            .get(&target)
            .into_iter()
            .flatten()
            .find(|x| x.observer == observer)
            .map_or(0, Registration::cost);
        let filter: Vec<String> = if names.is_empty() {
            Vec::new()
        } else {
            names.split('\n').map(str::to_owned).collect()
        };
        let r = Registration {
            observer,
            flags,
            filter,
        };
        let charge = r.cost();
        self.recharge(credit, charge)?;
        self.observer_bytes = (self.observer_bytes + charge).saturating_sub(credit);
        let list = self.registrations.entry(target).or_default();
        match list.iter_mut().find(|x| x.observer == observer) {
            Some(old) => *old = r,
            None => list.push(r),
        }
        Ok(())
    }

    /// `disconnect`: no registrations, no queued records.
    pub(crate) fn remove_registrations(&mut self, observer: u32) {
        let mut freed = 0;
        self.registrations.retain(|_, list| {
            list.retain(|r| {
                let keep = r.observer != observer;
                if !keep {
                    freed += r.cost();
                }
                keep
            });
            !list.is_empty()
        });
        self.credit(freed);
        self.observer_bytes = self.observer_bytes.saturating_sub(freed);
        if let Some(q) = self.record_queues.get_mut(observer as usize) {
            self.queued_records -= q.len();
            q.clear();
        }
    }

    /// The observer's oldest queued record, if any.
    pub(crate) fn next_record(&mut self, observer: u32) -> Option<Handle> {
        let h = self.record_queues.get_mut(observer as usize)?.pop_front()?;
        self.queued_records -= 1;
        Some(h)
    }

    pub(crate) fn has_records(&self) -> bool {
        self.queued_records > 0
    }

    /// Whether `id` is an HTML `<template>`.
    pub(crate) fn is_html_template(&self, id: usize) -> bool {
        self.element_name(id)
            .is_some_and(|n| n.ns == ns!(html) && n.local.as_ref() == "template")
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
        let held = self.bytes_held() + self.observer_bytes;
        for (r, range) in self.ranges.iter().enumerate() {
            for p in [range.start, range.end] {
                if self.inner.get_node(p.node).is_none() || !self.has_handle(p.node) {
                    return Err(format!("range {r} points at unpinned node {}", p.node));
                }
                if p.offset > self.node_length(p.node) {
                    return Err(format!("range {r} is past the end of node {}", p.node));
                }
            }
        }
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

/// What one attribute is charged.
fn attribute_cost(name: &QualName, value: &str) -> usize {
    ATTRIBUTE_COST + name_cost(name) + value.len()
}

/// URL's "percent-decode", then UTF-8 with replacement.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        let value = (bytes[i] == b'%')
            .then(|| Some(hex(*bytes.get(i + 1)?)? * 16 + hex(*bytes.get(i + 2)?)?))
            .flatten();
        match value {
            Some(v) => {
                out.push(v as u8);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

/// The prefix of an attribute's serialized name (HTML Standard §13.3,
/// "serializing HTML fragments"): none without a namespace; `xml`, `xmlns` or
/// `xlink` in those namespaces (none for the `xmlns` attribute itself); its
/// own prefix otherwise.
fn attribute_prefix(name: &QualName) -> Option<&str> {
    if name.ns == ns!() {
        None
    } else if name.ns == ns!(xml) {
        Some("xml")
    } else if name.ns == ns!(xmlns) {
        (name.local != *"xmlns").then_some("xmlns")
    } else if name.ns == ns!(xlink) {
        Some("xlink")
    } else {
        name.prefix.as_ref().map(|p| p.as_ref())
    }
}

/// Bytes of a string written between looks at the output's size and the clock.
const TEXT_SLICE: usize = 64 * 1024;

/// Nodes and slices between asks of a stop check.
const STOP_EVERY: u32 = 256;

/// Writes `s`, as it is if `raw` and escaped otherwise, in slices with a
/// `check` after each.
fn push_sliced(
    mut s: &str,
    raw: bool,
    attribute: bool,
    out: &mut String,
    check: &mut dyn FnMut(&str) -> Result<(), DomError>,
) -> Result<(), DomError> {
    while !s.is_empty() {
        let mut end = TEXT_SLICE.min(s.len());
        while !s.is_char_boundary(end) {
            end += 1;
        }
        if raw {
            out.push_str(&s[..end]);
        } else {
            escape_into(&s[..end], attribute, out);
        }
        s = &s[end..];
        if !s.is_empty() {
            check(out)?;
        }
    }
    Ok(())
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
