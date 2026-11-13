//! Raikiri as the layout engine behind `layout.rs` (E3-R1), with the
//! `raikiri` feature. `RAIKIRI.md` has the reasons; this file is the only
//! one that names a Raikiri type.
//!
//! As with Blitz, layout runs on a *mirror*: the page's tree copied into a
//! `raikiri_dom::Document` (one arena index per copied node, and one table
//! back to the page's node ids), cascaded, paginated, and read. Unlike the
//! Blitz mirror it doesn't outlive the layout: every answer a query can ask
//! for is read out at once into [`RaikiriLaid`], by page node id, and the
//! Raikiri document is dropped, so a laid-out page costs a few numbers per
//! node and not Raikiri's 10 KiB.
//!
//! Answers come from the union of a node's *paged* fragments, the boxes
//! Raikiri prints, never from its unfragmented layout, which gives table rows
//! no size, a box split across columns one column's width, and children of
//! `display: none` an origin (`RAIKIRI.md` section 2). Fragments are before
//! CSS transforms and before an inline element's relative offset, so both are
//! added here.
//!
//! Two choices this file makes (DECISIONS, E3-R1):
//!
//! - **Coordinates.** A script sees one continuous layout, as in a browser.
//!   A page's fragments are moved to where that page's content begins in the
//!   flow (`PageSlice::content_origin_y`), so a box after a page break is
//!   where the unbroken flow has it, not lower by the space left at the foot
//!   of every page before it. x and y are from the page area's corner, the
//!   initial containing block.
//! - **Media.** The measuring cascade runs as `screen`, with the page area as
//!   the viewport: what a Chrome-based tool's scripts measure, and what
//!   `matchMedia` answers while scripts run (DECISIONS 2026-11-11). Print
//!   rules apply only when the page is printed.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use blitz_dom::ns;
use raikiri_dom::{
    PageContentInsets, PageLayoutControl, PageLayoutOptions, PageMargins, PageSlice,
    layout_pages_with_options, resolve_page_geometry,
};
use raikiri_html::{
    FontCollectionBuilder, FragmentKind, MINIMAL_UA_CSS, MediaContext, MediaType, PageContextQuery,
    RenderFonts, StylesheetSource, UncascadedDocument, build_cascaded_with_options,
};
use raikiri_style::CascadeOptions;
use raikiri_style::property::PositionValue;
use raikiri_traits::{LayoutError, NodeId, PageBox, PaintRect, PaintTransform, StylesheetKind};
use surimono_webidl::runtime::{BindingError, BindingResult};

use crate::DomError;
use crate::document::{Document, Kind};
use crate::layout::{BoxRect, Engine, EngineError, EngineResult, Laid, LayoutSetup};
use crate::layout_blitz::{MAX_ATTRIBUTE_BYTES, MAX_SHEET_BYTES, MAX_TEXT_BYTES, copied};
use crate::sheet_screen::{Refused, Screen};
use crate::style_attr;

/// No mirror node.
const NONE: usize = usize::MAX;

/// fulgur-raikiri 0.41.0's user sheet (`DEFAULT_FONT_STYLESHEET` in its
/// `lib.rs`, private there): a page that names no family is set in
/// `sans-serif`, which Raikiri maps to the whole bundle in order.
const FONT_SHEET: &str = "@layer { :where(html) { font-family: sans-serif } }";

/// What the Raikiri engine needs beyond [`LayoutSetup`]'s viewport: the
/// fonts as Raikiri holds them, and the page. Internal, like the setup.
#[doc(hidden)]
#[derive(Clone)]
pub struct RaikiriSetup {
    fonts: RenderFonts,
    /// The page box in CSS px, and its margins (top, right, bottom, left),
    /// given to Raikiri as a user `@page` rule so that an author's `@page`
    /// wins, as when fulgur-raikiri prints. `None`: the setup's viewport as
    /// the page, with no margins.
    page: Option<((f32, f32), [f32; 4])>,
}

impl std::fmt::Debug for RaikiriSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaikiriSetup")
            .field("page", &self.page)
            .finish_non_exhaustive()
    }
}

impl RaikiriSetup {
    /// Fonts from their bytes, each with the family name it answers to, in
    /// the printer's order: Raikiri maps every generic family to the whole
    /// set in this order, so the font that should answer `sans-serif` goes
    /// first. The machine's fonts are never looked at.
    pub fn new<F, B>(fonts: impl IntoIterator<Item = (F, B)>) -> Result<Self, String>
    where
        F: Into<String>,
        B: Into<Arc<[u8]>>,
    {
        let mut builder = FontCollectionBuilder::new().system_fonts(false);
        for (family, bytes) in fonts {
            builder = builder.font_bytes(family, bytes);
        }
        let fonts = builder.build().map_err(|e| e.to_string())?;
        if !fonts.is_bundled_only() {
            return Err("the font set would use the machine's fonts".to_owned());
        }
        Ok(Self { fonts, page: None })
    }

    /// Lays out on a page of `width` by `height` CSS px with these margins
    /// (top, right, bottom, left), instead of the setup's viewport.
    pub fn with_page(mut self, width: f32, height: f32, margins: [f32; 4]) -> Self {
        self.page = Some(((width, height), margins));
        self
    }
}

/// The engine: see [`Engine`].
pub(crate) struct Raikiri;

impl Engine for Raikiri {
    fn lay_out(page: &Document, setup: &LayoutSetup) -> EngineResult {
        let Some(raikiri) = setup.raikiri() else {
            return Ok(None);
        };
        let Some(mirror) = build_mirror(page, raikiri)? else {
            return Ok(None);
        };
        let deadline = page.deadline.clone();
        let viewport = (setup.width, setup.height);
        guarded(move || lay_out_mirror(mirror, raikiri, viewport, deadline.as_ref()))
            .map(|laid| laid.map(|l| Box::new(l) as Box<dyn Laid>))
    }
}

/// Runs a layout. Raikiri is 0.0.x code that lays out whatever the page
/// holds: a panic in it leaves the page's tree whole (the layout only read
/// it), ends the call with a typed error, and takes the mirror with it.
fn guarded(
    f: impl FnOnce() -> Result<RaikiriLaid, Stop>,
) -> Result<Option<RaikiriLaid>, EngineError> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(laid)) => Ok(Some(laid)),
        Ok(Err(Stop::Refused)) => Ok(None),
        Ok(Err(Stop::TimeLimit)) => Err(time_limit().into()),
        Ok(Err(Stop::Failed(message))) => {
            Err(EngineError::Fault(DomError::LayoutFailed { message }))
        }
        Err(_) => Err(EngineError::Fault(DomError::LayoutFailed {
            message: "the layout engine panicked".to_owned(),
        })),
    }
}

fn time_limit() -> BindingError {
    BindingError::TypeError("surimono: the time limit has passed".to_owned())
}

/// Why a layout gave no laid-out page.
enum Stop {
    /// Over a bound Raikiri has of its own (pages, rules, fragments): answer
    /// as with no box, as for a page over our bounds.
    Refused,
    /// The page's deadline passed between two of Raikiri's stages.
    TimeLimit,
    /// Raikiri reported an error that isn't a bound.
    Failed(String),
}

/// The page's tree as a Raikiri document, before any style or layout.
struct Mirror {
    dom: raikiri_dom::Document,
    /// The page's `<style>` sheets in document order, with their `media`.
    sheets: Vec<StylesheetSource>,
    quirks: raikiri_traits::QuirksMode,
    /// Mirror arena index to page node id, [`NONE`] for the document node.
    back: Vec<usize>,
    /// Page node ids are below this.
    page_nodes: usize,
}

/// Copies the page's tree into a new Raikiri document. The copy looks at the
/// deadline per node and per style-attribute declaration; what goes into the
/// layout, which Raikiri can stop only between pages, is bounded first as
/// for Blitz: the page's node limit, [`MAX_TEXT_BYTES`],
/// [`MAX_ATTRIBUTE_BYTES`], [`MAX_SHEET_BYTES`] and the sheets' shape
/// (`sheet_screen.rs`). `None` for a page over any of them.
fn build_mirror(page: &Document, setup: &RaikiriSetup) -> BindingResult<Option<Mirror>> {
    let mut dom = raikiri_dom::Document::new();
    // Fonts before anything that can measure text (the cascade measures
    // `line-height: normal` for `lh`): a document with no fonts takes the
    // machine's on its first look (RAIKIRI.md section 4). The paragraphs are
    // then built on this thread only; giving a document fonts turns parallel
    // building back on, so this comes after.
    dom.set_font_collection(setup.fonts.collection().clone());
    dom.set_ifc_parallel_build(false);

    let quirks = page.page_quirks();
    let mut back = vec![NONE];
    let mut sheets: Vec<(usize, Option<String>)> = Vec::new();
    let mut text_bytes = 0usize;
    let mut attribute_bytes = 0usize;
    let mut screen = Screen::default();
    // (page node, its mirror parent), popped in document order.
    let mut stack: Vec<(usize, usize)> = page
        .children(page.root())
        .iter()
        .rev()
        .map(|&c| (c, 0))
        .collect();
    while let Some((live, parent)) = stack.pop() {
        page.poll()?;
        let id = match page.kind(live) {
            Some(Kind::Element) => {
                let Some(e) = page.node(live).element_data() else {
                    continue;
                };
                let mut style = None;
                let mut attrs = Vec::with_capacity(e.attrs().len());
                let mut namespaced = Vec::new();
                for a in e.attrs() {
                    let local = a.name.local.as_ref();
                    if a.name.ns == ns!() && local == "style" {
                        style = Some(a.value.as_str());
                        attribute_bytes = attribute_bytes.saturating_add(a.value.len());
                    } else {
                        let value = copied(local, &a.value);
                        attribute_bytes = attribute_bytes.saturating_add(value.len());
                        if a.name.ns == ns!() {
                            attrs.push((local.into(), value.into()));
                        } else {
                            namespaced.push((
                                a.name.ns.as_ref().to_owned(),
                                a.name.prefix.as_ref().map(|p| p.as_ref().into()),
                                local.to_owned(),
                                value.to_owned(),
                            ));
                        }
                    }
                    if attribute_bytes > MAX_ATTRIBUTE_BYTES {
                        return Ok(None);
                    }
                }
                if let Some(text) = style {
                    // The same screen a `style` read runs, a declaration at a
                    // time with the deadline looked at in between; Raikiri
                    // then parses the text as the printer will.
                    match screen_style_attribute(page, text, quirks, &mut screen) {
                        Ok(()) => {}
                        Err(style_attr::Stop::TimeLimit) => return Err(time_limit()),
                        Err(style_attr::Stop::TooLarge) => return Ok(None),
                    }
                }
                let local = e.name.local.as_ref();
                let id = dom.append_element(Some(parent), local, style);
                let namespace = (e.name.ns != ns!(html)).then(|| e.name.ns.as_ref().into());
                let prefix = e.name.prefix.as_ref().map(|p| p.as_ref().into());
                if namespace.is_some() || prefix.is_some() {
                    dom.set_element_namespace_info(id, namespace, prefix);
                }
                if !attrs.is_empty() {
                    dom.set_element_attributes(id, attrs);
                }
                for (namespace, prefix, local, value) in namespaced {
                    // One Raikiri refuses (a name no namespace allows)
                    // stays out of the copy; no selector can match it.
                    if dom
                        .set_element_namespaced_attribute(id, namespace, prefix, local, value)
                        .is_err()
                    {
                        continue;
                    }
                }
                let html = e.name.ns == ns!(html);
                let is_sheet = local == "style" && (html || e.name.ns == ns!(svg));
                // Children that are never rendered have no boxes, and their
                // text is not text a layout takes, as in the Blitz mirror.
                let unrendered = (html
                    && matches!(
                        local,
                        "img" | "canvas" | "script" | "title" | "noscript" | "template"
                    ))
                    || (e.name.ns == ns!(svg) && local == "script");
                if is_sheet {
                    if is_css(page.attribute(live, "type").as_deref()) {
                        sheets.push((live, page.attribute(live, "media")));
                    }
                } else if !unrendered {
                    stack.extend(page.children(live).iter().rev().map(|&c| (c, id)));
                }
                id
            }
            Some(Kind::Text) => {
                let text = page.char_data(live);
                text_bytes = text_bytes.saturating_add(text.len());
                if text_bytes > MAX_TEXT_BYTES {
                    return Ok(None);
                }
                dom.append_text(parent, text)
            }
            // Comments, doctypes and processing instructions have no box.
            _ => continue,
        };
        if id >= back.len() {
            back.resize(id + 1, NONE);
        }
        back[id] = live;
    }

    let mut sheet_bytes = 0usize;
    let texts: Vec<(String, Option<String>)> = sheets
        .into_iter()
        .map(|(live, media)| {
            let text = page
                .edited_sheet_text(live)
                .unwrap_or_else(|| page.style_text(live));
            sheet_bytes = sheet_bytes.saturating_add(text.len());
            (text, media)
        })
        .collect();
    if sheet_bytes > MAX_SHEET_BYTES {
        return Ok(None);
    }
    for (text, _) in &texts {
        screen.sheet(text, page.deadline.as_ref());
    }
    match screen.finish() {
        Ok(()) => {}
        Err(Refused::TooMuch) => return Ok(None),
        Err(Refused::TimeLimit) => return Err(time_limit()),
    }
    // As raikiri-html's parse does (`finish_document`): the UA sheet is the
    // document's own, and the user sheets come after it.
    dom.add_stylesheet(MINIMAL_UA_CSS, StylesheetKind::UserAgent);
    // The cascade reads the mode from the document, as the parse sets it.
    let quirks = match quirks {
        style::context::QuirksMode::Quirks => raikiri_traits::QuirksMode::Quirks,
        style::context::QuirksMode::LimitedQuirks => raikiri_traits::QuirksMode::LimitedQuirks,
        style::context::QuirksMode::NoQuirks => raikiri_traits::QuirksMode::NoQuirks,
    };
    dom.set_quirks_mode(quirks);
    dom.mark_in_document_flags();
    Ok(Some(Mirror {
        dom,
        sheets: texts
            .into_iter()
            .map(|(text, media)| StylesheetSource::new(text, media))
            .collect(),
        quirks,
        back,
        page_nodes: page.node_count(),
    }))
}

/// HTML §4.2.6: a `<style>` is a CSS sheet when its `type` is absent, empty
/// or `text/css`, ignoring ASCII case.
fn is_css(kind: Option<&str>) -> bool {
    kind.is_none_or(|t| t.is_empty() || t.eq_ignore_ascii_case("text/css"))
}

/// A style attribute through the parse a `style` read does, for its bounds
/// and the deadline: what [`style_attr::fits`] let in bounds the work, and
/// `screen` hears which custom properties name which.
fn screen_style_attribute(
    page: &Document,
    text: &str,
    quirks: style::context::QuirksMode,
    screen: &mut Screen,
) -> Result<(), style_attr::Stop> {
    for d in style_attr::parse(text, quirks, page.deadline.as_ref())? {
        if d.value.len() <= style_attr::MAX_CHECKED_VALUE {
            screen.attribute_declaration(&d.name, &d.value);
        }
    }
    Ok(())
}

/// The page box and its margins: the setup's, or its viewport with none.
fn page_of(setup: &RaikiriSetup, viewport: (u32, u32)) -> (PageBox, [f32; 4]) {
    let ((width, height), margins) = setup
        .page
        .unwrap_or(((viewport.0 as f32, viewport.1 as f32), [0.0; 4]));
    let mut page = PageBox::new();
    page.width = width;
    page.height = height;
    (page, margins)
}

/// Cascades and paginates the mirror and reads every box out of it.
fn lay_out_mirror(
    mirror: Mirror,
    setup: &RaikiriSetup,
    viewport: (u32, u32),
    deadline: Option<&surimono_core::Deadline>,
) -> Result<RaikiriLaid, Stop> {
    let passed = || deadline.is_some_and(|d| d.passed());
    let Mirror {
        dom,
        sheets,
        quirks,
        back,
        page_nodes,
    } = mirror;
    let (default_page, [top, right, bottom, left]) = page_of(setup, viewport);
    let page_sheet = format!("@page {{ margin: {top}px {right}px {bottom}px {left}px; }}");
    let doc = UncascadedDocument::new(
        dom,
        sheets,
        vec![
            StylesheetSource::new(FONT_SHEET.to_owned(), None),
            StylesheetSource::new(page_sheet, None),
        ],
        1,
        quirks,
    );
    // The first page's context: `:first`, and `:right` for a left-to-right
    // page progression, as raikiri-html's pre-layout cascades use.
    let mut first = PageContextQuery::default();
    first.is_first = true;
    first.is_right = true;
    let mut options = CascadeOptions::default();
    // As fulgur-raikiri prints: border widths snapped as on a 1x device.
    options.border_width_device_scale = Some(1.0);

    // The page area is known only from the page context, which the first
    // cascade gives; a second one runs with it as the viewport.
    let cascade = |options: &CascadeOptions, media: &MediaContext| {
        build_cascaded_with_options(&doc, media, &first, &[], options).map_err(|e| match e {
            // Rules, selectors or declarations over the cascade's own bounds,
            // or more than it can allocate: a page too big, as for ours.
            raikiri_style::CascadeError::LimitExceeded { .. }
            | raikiri_style::CascadeError::ResourceExhausted { .. } => Stop::Refused,
            other => Stop::Failed(other.to_string()),
        })
    };
    if passed() {
        return Err(Stop::TimeLimit);
    }
    let provisional = cascade(&options, &screen(default_page))?;
    let page_box = PageBox::from_page_size_or(provisional.page.size(), default_page);
    let geometry = resolve_page_geometry(&provisional.page, page_box);
    let area = geometry.content_box;
    options.viewport = Some((area.width, area.height));
    let media = MediaContext::with_viewport(
        MediaType::Screen,
        area.width.max(0.0) as u32,
        area.height.max(0.0) as u32,
    );
    if passed() {
        return Err(Stop::TimeLimit);
    }
    let cascade = cascade(&options, &media)?;
    drop(provisional);
    let UncascadedDocument { mut dom, .. } = doc;

    let abort = || passed();
    let control =
        PageLayoutControl::new(raikiri_traits::RenderLimits::default().max_document_pages)
            .with_abort_check(&abort);
    let slices = layout_pages_with_options(
        &mut dom,
        &cascade,
        page_box,
        PageLayoutOptions {
            control: Some(&control),
            ..Default::default()
        },
    )
    .map_err(stop)?;
    // Every page has the first page's geometry: the measuring copy doesn't
    // follow `@page :left`, `:right` or named pages (RAIKIRI.md section 8).
    let geometries: Vec<(PageBox, PageMargins, PageContentInsets)> = slices
        .iter()
        .map(|_| (geometry.page_box, geometry.margins, geometry.content_insets))
        .collect();
    dom.project_pages_with_control(&cascade, page_box, &slices, &geometries, &control)
        .map_err(stop)?;
    if passed() {
        return Err(Stop::TimeLimit);
    }
    Ok(read_boxes(&dom, &cascade, &slices, area, &back, page_nodes))
}

fn screen(page: PageBox) -> MediaContext {
    MediaContext::with_viewport(
        MediaType::Screen,
        page.width.max(0.0) as u32,
        page.height.max(0.0) as u32,
    )
}

fn stop(e: LayoutError) -> Stop {
    match e {
        LayoutError::Aborted => Stop::TimeLimit,
        // Raikiri's own bounds: pages, fragments, a paragraph's size, 256
        // levels of boxes, counters; and what its paragraphs can't lay out.
        LayoutError::PageLimitExceeded { .. }
        | LayoutError::FragmentLimitExceeded { .. }
        | LayoutError::IfcLimitExceeded { .. }
        | LayoutError::IfcUnsupported { .. }
        | LayoutError::TreeDepthLimitExceeded { .. }
        | LayoutError::CounterSnapshotLimitExceeded { .. } => Stop::Refused,
        other => Stop::Failed(other.to_string()),
    }
}

/// A page Raikiri laid out, as answers by page node id.
#[derive(Debug, Default)]
struct RaikiriLaid {
    /// `getBoundingClientRect`: the union of the node's fragments, each
    /// moved by its relative offsets and transforms, in continuous
    /// coordinates.
    boxes: Vec<Option<BoxRect>>,
    /// `offsetLeft` and `offsetTop` (CSSOM View §7): the first fragment's
    /// border edge, transforms ignored, less the offset parent's padding
    /// edge.
    offsets: Vec<Option<(f64, f64)>>,
}

impl Laid for RaikiriLaid {
    fn border_box(&self, node: usize) -> Option<BoxRect> {
        self.boxes.get(node).copied().flatten()
    }

    fn offset_position(&self, node: usize) -> Option<(f64, f64)> {
        self.offsets.get(node).copied().flatten()
    }
}

/// The bounding box of `rect` moved by `t`.
fn transformed(rect: PaintRect, t: PaintTransform) -> PaintRect {
    let corners = [
        t.apply((rect.x, rect.y)),
        t.apply((rect.x + rect.width, rect.y)),
        t.apply((rect.x, rect.y + rect.height)),
        t.apply((rect.x + rect.width, rect.y + rect.height)),
    ];
    let (mut l, mut tp, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        l = l.min(x);
        tp = tp.min(y);
        r = r.max(x);
        b = b.max(y);
    }
    PaintRect::new(l, tp, r - l, b - tp)
}

fn union(a: Option<BoxRect>, b: BoxRect) -> BoxRect {
    let Some(a) = a else { return b };
    let (l, t) = (a.x.min(b.x), a.y.min(b.y));
    let (r, bo) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    BoxRect {
        x: l,
        y: t,
        width: r - l,
        height: bo - t,
    }
}

/// Reads every element's boxes out of the paged layout, then the offsets.
fn read_boxes(
    dom: &raikiri_dom::Document,
    cascade: &raikiri_html::CascadeResult,
    slices: &[PageSlice],
    area: PaintRect,
    back: &[usize],
    page_nodes: usize,
) -> RaikiriLaid {
    let n = dom.node_count();
    // Relative offsets of inline elements, which no fragment includes: each
    // node moves with its own and every ancestor's (`Node::ifc_relative_offsets`
    // of each paragraph root, by node). Parents come before children in the
    // arena, so one pass in index order adds them up.
    let mut moved = vec![(0.0f32, 0.0f32); n];
    for i in 0..n {
        if let Some(node) = dom.get_node(i) {
            for &(id, (dx, dy)) in node.ifc_relative_offsets() {
                if let Some(m) = moved.get_mut(id) {
                    m.0 += dx;
                    m.1 += dy;
                }
            }
        }
    }
    for i in 1..n {
        if let Some(p) = dom.parent_of(i)
            && p < i
        {
            let (px, py) = moved[p];
            moved[i].0 += px;
            moved[i].1 += py;
        }
    }

    let mut boxes: Vec<Option<BoxRect>> = vec![None; n];
    let mut firsts: Vec<Option<BoxRect>> = vec![None; n];
    for slice in slices {
        // Page coordinates to the continuous flow's: from the page area's
        // corner, and down by where this page's content begins in the flow.
        let dx = -f64::from(area.x);
        let dy = f64::from(slice.content_origin_y) - f64::from(area.y);
        let fragments: Vec<_> = dom
            .page_fragments(slice.page_index)
            .filter(|f| !matches!(f.kind(), FragmentKind::Text))
            .map(|f| (f.node(), f.rect(), f.repeat().is_some(), f.fragment_index()))
            .collect();
        let mut nodes: Vec<NodeId> = fragments.iter().map(|f| f.0).collect();
        nodes.sort_by_key(|id| id.0);
        nodes.dedup();
        let transforms =
            dom.page_paint_transforms(cascade, slice.page_index, nodes.iter().copied());
        for (node, rect, repeat, index) in fragments {
            let Ok(i) = usize::try_from(node.0) else {
                continue;
            };
            if i >= n || (repeat && boxes[i].is_some()) {
                continue;
            }
            let (mx, my) = moved[i];
            let rect = PaintRect::new(rect.x + mx, rect.y + my, rect.width, rect.height);
            let to_flow = |r: PaintRect| BoxRect {
                x: f64::from(r.x) + dx,
                y: f64::from(r.y) + dy,
                width: f64::from(r.width),
                height: f64::from(r.height),
            };
            if index == 0 && firsts[i].is_none() {
                firsts[i] = Some(to_flow(rect));
            }
            let at = nodes.binary_search(&node).ok();
            let shown = match at.and_then(|k| transforms.get(k)) {
                // A singular transform: drawn nowhere, so no box from this.
                Some(None) => continue,
                Some(Some(t)) => transformed(rect, *t),
                None => rect,
            };
            boxes[i] = Some(union(boxes[i], to_flow(shown)));
        }
    }

    // CSSOM View §7, offsetParent and offsetTop, as Chromium 153 answers
    // them (tests/data/chrome-layout/offset-parents.chrome.json): the
    // nearest ancestor that is positioned, or a containing block for
    // positioned descendants (a transform or a filter), or `body`, or for a
    // static element a `td`, `th` or `table`; none for the root, `body`, a
    // fixed element or an element with no box. Offsets are from the offset
    // parent's padding edge, transforms ignored, except from `body`, whose
    // own position Chrome never takes off: from the initial containing
    // block, as with no offset parent.
    let mut out = RaikiriLaid {
        boxes: vec![None; page_nodes],
        offsets: vec![None; page_nodes],
    };
    let computed = |i: usize| cascade.computed.get(i);
    let position_is =
        |i: usize, want: fn(&PositionValue) -> bool| computed(i).is_some_and(|c| want(&c.position));
    let contains_positioned = |i: usize| {
        computed(i).is_some_and(|c| {
            !matches!(c.position, PositionValue::Static)
                || !c.transform.is_empty()
                || !c.filter.is_empty()
        })
    };
    let html = |i: usize, names: &[&str]| {
        dom.get_node(i).is_some_and(|node| {
            node.namespace_uri().is_none() && node.tag_name().is_some_and(|t| names.contains(&t))
        })
    };
    let parent_of = |i: usize| dom.parent_of(i).filter(|&p| p != 0);
    for i in 1..n {
        let Some(&live) = back.get(i) else { continue };
        if live == NONE || live >= page_nodes {
            continue;
        }
        out.boxes[live] = boxes[i];
        let Some(first) = firsts[i] else { continue };
        if html(i, &["body"]) {
            // §7 offsetTop step 1.
            out.offsets[live] = Some((0.0, 0.0));
            continue;
        }
        let fixed = position_is(i, |p| matches!(p, PositionValue::Fixed));
        let static_self = position_is(i, |p| matches!(p, PositionValue::Static));
        let mut found = None;
        if !fixed {
            let mut parent = parent_of(i);
            while let Some(p) = parent {
                if contains_positioned(p)
                    || html(p, &["body"])
                    || (static_self && html(p, &["td", "th", "table"]))
                {
                    found = Some(p);
                    break;
                }
                parent = parent_of(p);
            }
        }
        out.offsets[live] = Some(match found.filter(|&p| !html(p, &["body"])) {
            Some(p) => {
                let edge = firsts[p].unwrap_or(first);
                let border = dom.get_node(p).map(|node| node.layout().border);
                let (bl, bt) = border.map_or((0.0, 0.0), |b| (f64::from(b.left), f64::from(b.top)));
                (first.x - (edge.x + bl), first.y - (edge.y + bt))
            }
            None => (first.x, first.y),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_in_the_engine_is_a_typed_fault() {
        let got = guarded(|| panic!("inside the engine"));
        assert!(
            matches!(&got, Err(EngineError::Fault(DomError::LayoutFailed { message }))
                if message == "the layout engine panicked"),
            "{got:?}"
        );
        let got = guarded(|| Err(Stop::Failed("taffy".to_owned())));
        assert!(matches!(
            got,
            Err(EngineError::Fault(DomError::LayoutFailed { .. }))
        ));
        assert!(matches!(guarded(|| Err(Stop::Refused)), Ok(None)));
        assert!(matches!(
            guarded(|| Err(Stop::TimeLimit)),
            Err(EngineError::Binding(BindingError::TypeError(_)))
        ));
    }

    #[test]
    fn a_rectangle_moves_with_its_transform() {
        let r = PaintRect::new(0.0, 20.0, 50.0, 10.0);
        let t = transformed(r, PaintTransform::translate(10.0, 3.0));
        assert_eq!((t.x, t.y, t.width, t.height), (10.0, 23.0, 50.0, 10.0));
        // scale(2) about the corner, and a quarter turn: the bounding box.
        let t = transformed(r, PaintTransform::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0));
        assert_eq!((t.x, t.y, t.width, t.height), (0.0, 40.0, 100.0, 20.0));
        let t = transformed(r, PaintTransform::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0));
        assert_eq!((t.x, t.y, t.width, t.height), (-30.0, 0.0, 10.0, 50.0));
    }

    #[test]
    fn only_css_style_elements_are_sheets() {
        for css in [None, Some(""), Some("text/css"), Some("TEXT/CSS")] {
            assert!(is_css(css), "{css:?}");
        }
        for other in [
            Some("text/plain"),
            Some("text/css; charset=utf-8"),
            Some(" text/css"),
        ] {
            assert!(!is_css(other), "{other:?}");
        }
    }
}
