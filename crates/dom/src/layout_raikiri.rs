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
//! - **Coordinates.** A script sees one continuous layout, as in a browser:
//!   the page's width and height are the viewport (the initial containing
//!   block, `vw` and `vh`, media queries), but nothing is cut at the page's
//!   end. Raikiri still paginates, into one page [`UNBROKEN`] px long, so
//!   that the answers are its paged fragments, and what only a paged medium
//!   has is taken off the measuring copy (`sheet_unpaged.rs` for the style
//!   text, [`unpaged`] for the cascade): a page break does nothing on a
//!   screen. So the paged positions *are* the continuous ones, and nothing
//!   is mapped back. The page begins [`ABOVE`] px over the flow's start,
//!   since a script measures there too. x and y are from the page area's
//!   corner, the initial containing block's. A page with a box as far out as
//!   Raikiri lays out ([`FARTHEST`]) is not laid out.
//! - **Media.** The measuring cascade runs as `screen`: the media type a
//!   Chrome-based tool's scripts see, and what `matchMedia` answers while
//!   scripts run (DECISIONS 2026-11-11). Print rules apply only when the
//!   page is printed. The viewport is the page area, which is ours: with an
//!   author `@page` size it isn't the window a Chrome-based tool has.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use blitz_dom::ns;
use raikiri_dom::{
    PageLayoutControl, PageLayoutOptions, PageSlice, layout_pages_with_options,
    resolve_page_geometry,
};
use raikiri_html::{
    FontCollectionBuilder, FragmentKind, MINIMAL_UA_CSS, MediaContext, MediaType, PageContextQuery,
    RenderFonts, RepeatKind, StylesheetSource, UncascadedDocument, build_cascaded_with_options,
};
use raikiri_style::property::{
    BreakBetween, ContentComponent, DisplayValue, FloatValue, PageValue, PositionValue, WillChange,
};
use raikiri_style::{
    CascadeOptions, ComputedLength, ComputedLineHeight, ComputedValues, PseudoValueArc,
};
use raikiri_traits::{LayoutError, NodeId, PageBox, PaintRect, PaintTransform, StylesheetKind};
use surimono_webidl::runtime::BindingError;

use crate::DomError;
use crate::document::{Document, Kind};
use crate::layout::{BoxRect, Engine, EngineError, EngineResult, Laid, LayoutRefusal, LayoutSetup};
use crate::layout_blitz::{MAX_ATTRIBUTE_BYTES, MAX_SHEET_BYTES, MAX_TEXT_BYTES, copied};
use crate::sheet_screen::{Refused, Screen};
use crate::sheet_unpaged::{unpaged as unpaged_sheet, unpaged_attribute};
use crate::style_attr;

/// No mirror node.
const NONE: usize = usize::MAX;

/// fulgur-raikiri 0.41.0's user sheet (`DEFAULT_FONT_STYLESHEET` in its
/// `lib.rs`, private there): a page that names no family is set in
/// `sans-serif`, which Raikiri maps to the whole bundle in order.
const FONT_SHEET: &str = "@layer { :where(html) { font-family: sans-serif } }";

/// The length of the one page the measuring copy is paginated into, in CSS
/// px: longer than any page Raikiri lays out, since it keeps every length
/// and position within [`FARTHEST`].
const UNBROKEN: f32 = 16_777_216.0;

/// How far above the page area's top the one page begins, in CSS px.
/// Raikiri's fragments begin at their page's top: a box above it is cut
/// there or has no fragment. A script measures text there (`position:
/// absolute; top: -9999px`), so the page is made to begin this far up; a page
/// with a box cut at this line is not laid out ([`place`]), and a box wholly
/// above it has no box. A power of two, and no larger, so that the
/// first 131,072 px below the top keep the 1/64 px Chromium lays out in.
const ABOVE: f32 = 131_072.0;

/// Raikiri keeps every length and position within 10,000,000 px of zero
/// (`MAX_TAFFY_MAGNITUDE`), silently: a longer box ends there and a later one
/// begins there. A page with a box that reaches it is not laid out.
const FARTHEST: f64 = 1e7;

/// Most nodes (elements and text) one layout takes. Raikiri's layout peaks at
/// 10 to 13 KiB a node (RAIKIRI.md section 10), so this keeps one layout
/// near 260 MiB at its peak; the page's own `max_nodes` (100,000) would be
/// over 1 GiB. It does not bound a layout's time, which isn't a matter of
/// the node count alone: the deadline does, through [`bounded`], with the
/// estimate [`too_costly`].
pub(crate) const MAX_LAYOUT_NODES: usize = 20_000;

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

/// Raikiri layout threads alive in the process, running or given up on at
/// the deadline ([`bounded`]). Internal: for the tests of the time bound.
#[doc(hidden)]
pub fn layout_threads() -> usize {
    bounded::live()
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
        let paper = page_of(raikiri, (setup.width, setup.height));
        let theirs = deadline.clone();
        // The cascade and the layout pass can't be stopped once they run
        // (RAIKIRI.md section 10): they run on a thread of their own, which
        // the call stops waiting for at the deadline.
        let laid = bounded::run(deadline.as_ref(), move || {
            guarded(move || lay_out_mirror(mirror, paper, theirs.as_ref()))
        })??;
        Ok(Some(Box::new(laid) as Box<dyn Laid>))
    }
}

/// Runs a layout. Raikiri is 0.0.x code that lays out whatever the page
/// holds: a panic in it leaves the page's tree whole (the layout only read
/// it), ends the call with a typed error, and takes the mirror with it.
fn guarded(f: impl FnOnce() -> Result<RaikiriLaid, Stop>) -> Result<RaikiriLaid, EngineError> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(laid)) => Ok(laid),
        Ok(Err(Stop::Refused(reason))) => Err(EngineError::Refused(reason)),
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
    /// Over a bound (ours, or one Raikiri has of its own: pages, rules,
    /// fragments): answer as with no box, and count why.
    Refused(LayoutRefusal),
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
/// layout, which Raikiri can stop only between pages (and the measuring copy
/// is one page), is bounded first: [`MAX_LAYOUT_NODES`], and as for Blitz
/// [`MAX_TEXT_BYTES`], [`MAX_ATTRIBUTE_BYTES`], [`MAX_SHEET_BYTES`] and the
/// sheets' shape (`sheet_screen.rs`). [`EngineError::Refused`] for a page
/// over any of them, `None` for a document with no root element.
fn build_mirror(page: &Document, setup: &RaikiriSetup) -> Result<Option<Mirror>, EngineError> {
    let refused = |r| Err(EngineError::Refused(r));
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
    let mut copied_nodes = 0usize;
    let mut has_root_element = false;
    while let Some((live, parent)) = stack.pop() {
        page.poll()?;
        if matches!(page.kind(live), Some(Kind::Element | Kind::Text)) {
            copied_nodes += 1;
            if copied_nodes > MAX_LAYOUT_NODES {
                return refused(LayoutRefusal::Nodes);
            }
        }
        has_root_element |= parent == 0 && page.kind(live) == Some(Kind::Element);
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
                        return refused(LayoutRefusal::Attributes);
                    }
                }
                if let Some(text) = style {
                    // The same screen a `style` read runs, a declaration at a
                    // time with the deadline looked at in between; Raikiri
                    // then parses the text as the printer will.
                    match screen_style_attribute(page, text, quirks, &mut screen) {
                        Ok(()) => {}
                        Err(style_attr::Stop::TimeLimit) => return Err(time_limit().into()),
                        Err(style_attr::Stop::TooLarge) => {
                            return refused(LayoutRefusal::Attributes);
                        }
                    }
                }
                // Raikiri is given what a browser would keep of it.
                let style = match style.map(|s| unpaged_attribute(s, page.deadline.as_ref())) {
                    Some(Err(_)) => return Err(time_limit().into()),
                    Some(Ok(kept)) => Some(kept),
                    None => None,
                };
                let local = e.name.local.as_ref();
                let id = dom.append_element(Some(parent), local, style.as_deref());
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
                    return refused(LayoutRefusal::Text);
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

    // A document a script left without a root element has no boxes, and
    // Raikiri lays out only a document that has one.
    if !has_root_element {
        return Ok(None);
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
        return refused(LayoutRefusal::Sheets);
    }
    for (text, _) in &texts {
        screen.sheet(text, page.deadline.as_ref());
    }
    match screen.finish() {
        Ok(()) => {}
        Err(Refused::TooMuch) => return refused(LayoutRefusal::Sheets),
        Err(Refused::TimeLimit) => return Err(time_limit().into()),
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
    // What a browser would keep of each sheet.
    let mut kept_sheets = Vec::with_capacity(texts.len());
    for (text, media) in texts {
        let text = match unpaged_sheet(&text, page.deadline.as_ref()) {
            Err(_) => return Err(time_limit().into()),
            Ok(std::borrow::Cow::Owned(kept)) => kept,
            Ok(std::borrow::Cow::Borrowed(_)) => text,
        };
        kept_sheets.push(StylesheetSource::new(text, media));
    }
    Ok(Some(Mirror {
        dom,
        sheets: kept_sheets,
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

/// Cascades and paginates the mirror and reads every box out of it, on the
/// layout's own thread ([`bounded`]). `paper` is the page box and its
/// margins ([`page_of`]).
fn lay_out_mirror(
    mirror: Mirror,
    paper: (PageBox, [f32; 4]),
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
    let (default_page, [top, right, bottom, left]) = paper;
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
            | raikiri_style::CascadeError::ResourceExhausted { .. } => {
                Stop::Refused(LayoutRefusal::EngineBound)
            }
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
    let mut cascade = cascade(&options, &media)?;
    drop(provisional);
    unpaged(&mut cascade);
    let UncascadedDocument { mut dom, .. } = doc;
    if lays_out_too_far(&dom, &cascade) {
        return Err(Stop::Refused(LayoutRefusal::TooFar));
    }
    if too_costly(&dom, &cascade) {
        return Err(Stop::Refused(LayoutRefusal::Cost));
    }

    let abort = || passed();
    let control =
        PageLayoutControl::new(raikiri_traits::RenderLimits::default().max_document_pages)
            .with_abort_check(&abort);
    // One page, UNBROKEN long, of the page area's width: the layout itself
    // runs against `page_box`, so percentages, `bottom` and fixed boxes see
    // the page's height, and only the cut into pages sees the long one.
    let slices = layout_pages_with_options(
        &mut dom,
        &cascade,
        page_box,
        PageLayoutOptions {
            page_steps: &[UNBROKEN],
            control: Some(&control),
            ..Default::default()
        },
    )
    .map_err(stop)?;
    // What is longer than the page is over Raikiri's own bound on lengths;
    // it isn't expected, and nothing here reads a second page.
    let [slice] = slices.as_slice() else {
        return Err(Stop::Refused(LayoutRefusal::EngineBound));
    };
    // The fragments are projected onto a page that begins ABOVE px over the
    // flow's start and is as long as its step from there.
    let mut project = |origin: f32| -> Result<Placed, Stop> {
        let slice = PageSlice {
            content_origin_y: origin,
            ..slice.clone()
        };
        let mut long = geometry.page_box;
        long.height = long.height - area.height - origin + UNBROKEN;
        dom.project_pages_with_control(
            &cascade,
            page_box,
            std::slice::from_ref(&slice),
            &[(long, geometry.margins, geometry.content_insets)],
            &control,
        )
        .map_err(stop)?;
        if passed() {
            return Err(Stop::TimeLimit);
        }
        // On the page that begins ABOVE px up, a box that begins at the
        // page's top was cut there: it reaches further up than the page.
        let top = (origin < 0.0).then_some(f64::from(origin));
        place(&dom, &cascade, &slice, area, top).map_err(Stop::Refused)
    };
    let mut placed = project(-ABOVE)?;
    // Raikiri places a table's header and footer groups by the flow position
    // their page begins at, so they are on no page that begins elsewhere:
    // a page with one is projected again as Raikiri cut it, and what has no
    // box yet takes its box from there.
    let groups = |c: &ComputedValues| {
        matches!(
            c.display,
            DisplayValue::TableHeaderGroup | DisplayValue::TableFooterGroup
        )
    };
    if cascade.computed.iter().any(groups) {
        let at_top = project(0.0)?;
        placed.fill_from(at_top);
    }
    Ok(read_boxes(&dom, &cascade, &placed, &back, page_nodes))
}

/// Takes what only a paged medium has off the measuring cascade, so that the
/// page is laid out as a browser lays it out for a script:
///
/// - forced page breaks and named pages, on elements and on pseudo-elements,
///   which do nothing in a continuous layout (CSS Fragmentation 3 §3.1: the
///   page values apply to paged media). A forced column break stays one:
///   `always` forces a break in the nearest fragmentation context, which
///   without pages is a column or nothing;
/// - `float: footnote` and `position: running()` (CSS GCPM 3) that reach the
///   cascade although `sheet_unpaged.rs` cut or renamed them in the text
///   (`--f: var(--x, footnote)`): the element is in the flow, with the
///   display Raikiri blockified it to;
/// - a pseudo-element whose `content` a browser drops for what is in it:
///   `string()`, `element()`, `target-counter()`, `target-counters()` and
///   `target-text()`. It then has no content of its own and no box.
fn unpaged(cascade: &mut raikiri_html::CascadeResult) {
    for computed in &mut cascade.computed {
        unpaged_values(computed);
        snap_line_height(computed);
    }
    for page in &mut cascade.page_values {
        *page = PageValue::Auto;
    }
    cascade
        .pseudo
        .retain(|_, computed| !computed.content.iter().any(paged_only));
    for computed in cascade.pseudo.values_mut() {
        if is_paged(computed) {
            unpaged_values(PseudoValueArc::make_mut(computed));
        }
        if snapped_line_height(computed).is_some() {
            snap_line_height(PseudoValueArc::make_mut(computed));
        }
    }
}

/// The line height as Chromium 153 lays lines out, which is in whole 1/64 px:
/// a number times the font size (in 1/64 px, the nearest) rounded down to
/// one, a length (`em`, `%` and `calc()` included) rounded to the nearest
/// (the `line-height*` and `article` probes: `13px/1.4` is 18.1875 px a line,
/// `line-height: 1.3em` at 14px 18.203125). Raikiri keeps the exact length,
/// so without this the page drifts from Chromium's by a pixel every 30 to 60
/// lines. A rule fitted to the probes' lines of one font size: Chromium's
/// line box, which this doesn't model, differs from it for a span of another
/// size on a line with a length line height and for some percentages
/// (RAIKIRI.md section 9). `normal` is left alone; it is whole pixels in
/// both. `None` where nothing changes.
fn snapped_line_height(computed: &ComputedValues) -> Option<ComputedLineHeight> {
    let snapped = match computed.line_height {
        ComputedLineHeight::Normal => return None,
        ComputedLineHeight::Number(n) => {
            // Of the font size in 1/64 px, the nearest (`14.3px/1.25` is
            // 14.296875 times 1.25, rounded down: 17.859375). In f32, as
            // Raikiri multiplies, and a hair over: `20px/1.4` is 28 px, not
            // the 27.99999952 the f32 for 1.4 times 20 is.
            let size = (computed.font_size.px() * 64.0).round() / 64.0;
            let px = n * size;
            (f64::from(px) * 64.0 + 1e-4).floor() / 64.0
        }
        ComputedLineHeight::Length(l) => {
            let snapped = (f64::from(l.px()) * 64.0).round() / 64.0;
            if snapped == f64::from(l.px()) {
                return None;
            }
            snapped
        }
    };
    // A length no f32 holds stays as Raikiri has it.
    let snapped = snapped as f32;
    snapped
        .is_finite()
        .then_some(ComputedLineHeight::Length(ComputedLength(snapped)))
}

fn snap_line_height(computed: &mut ComputedValues) {
    if let Some(snapped) = snapped_line_height(computed) {
        computed.line_height = snapped;
    }
}

/// Whether [`unpaged_values`] would change `computed`.
fn is_paged(computed: &ComputedValues) -> bool {
    let forced = |value: BreakBetween| matches!(value, BreakBetween::Page | BreakBetween::Always);
    forced(computed.break_before)
        || forced(computed.break_after)
        || matches!(computed.float, FloatValue::Footnote)
        || !computed.running_templates.is_empty()
}

fn unpaged_values(computed: &mut ComputedValues) {
    let continuous = |value: &mut BreakBetween| match *value {
        BreakBetween::Page => *value = BreakBetween::Auto,
        BreakBetween::Always => *value = BreakBetween::Column,
        _ => {}
    };
    continuous(&mut computed.break_before);
    continuous(&mut computed.break_after);
    if matches!(computed.float, FloatValue::Footnote) {
        computed.float = FloatValue::None;
    }
    // A running element computes to `display: none` and keeps the display
    // it would have had in its template.
    if let Some(template) = computed.running_templates.pop() {
        computed.display = template.display;
        computed.running_templates.clear();
    }
}

/// Whether a browser drops a `content` declaration for this part of it.
fn paged_only(component: &ContentComponent) -> bool {
    matches!(
        component,
        ContentComponent::String { .. }
            | ContentComponent::Element { .. }
            | ContentComponent::TargetCounter { .. }
            | ContentComponent::TargetCounters { .. }
            | ContentComponent::TargetText { .. }
    )
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
        | LayoutError::CounterSnapshotLimitExceeded { .. } => {
            Stop::Refused(LayoutRefusal::EngineBound)
        }
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
    /// `offsetLeft` and `offsetTop`: the first fragment's border edge,
    /// transforms ignored, less the edge Chromium measures from
    /// ([`read_boxes`]), which for `body` is not CSSOM View §7's.
    offsets: Vec<Option<(f64, f64)>>,
    /// `offsetWidth` and `offsetHeight`: the size of the union of the node's
    /// fragments, transforms ignored.
    sizes: Vec<Option<(f64, f64)>>,
}

impl Laid for RaikiriLaid {
    fn border_box(&self, node: usize) -> Option<BoxRect> {
        self.boxes.get(node).copied().flatten()
    }

    fn offset_position(&self, node: usize) -> Option<(f64, f64)> {
        self.offsets.get(node).copied().flatten()
    }

    fn offset_size(&self, node: usize) -> Option<(f64, f64)> {
        self.sizes.get(node).copied().flatten()
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

fn is_finite(r: BoxRect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite()
}

/// What the fragments say of each mirror node, in continuous coordinates.
struct Placed {
    /// The union of the node's fragments as the page shows them: moved by
    /// relative offsets and by transforms.
    shown: Vec<Option<BoxRect>>,
    /// The same union before any transform.
    laid: Vec<Option<BoxRect>>,
    /// Where the node's first box begins, before any transform: its first
    /// fragment's corner, or for an inline element on several lines its
    /// first line's.
    first: Vec<Option<(f64, f64)>>,
}

impl Placed {
    /// Takes from `other` what it has of every node this has no box of.
    fn fill_from(&mut self, other: Placed) {
        let others = other.shown.into_iter().zip(other.laid).zip(other.first);
        for (i, ((shown, laid), first)) in others.enumerate() {
            if i < self.laid.len() && self.laid[i].is_none() {
                self.shown[i] = shown;
                self.laid[i] = laid;
                self.first[i] = first;
            }
        }
    }
}

/// Reads every node's fragments out of the one page of the layout. Refused
/// if a box reaches [`FARTHEST`], where Raikiri's numbers stop being the
/// page's, or, with `top` (the page's top in flow coordinates), if a box in
/// the flow begins at or above it, where Raikiri cut it: its numbers aren't
/// the page's either. A box wholly above `top` has no fragment, and so no
/// box.
fn place(
    dom: &raikiri_dom::Document,
    cascade: &raikiri_html::CascadeResult,
    slice: &PageSlice,
    area: PaintRect,
    top: Option<f64>,
) -> Result<Placed, LayoutRefusal> {
    let n = dom.node_count();
    // Relative offsets of inline elements, which no fragment includes: each
    // node moves with its own and every ancestor's (`Node::ifc_relative_offsets`
    // of each paragraph root, by node). Parents come before children in the
    // arena, so one pass in index order adds them up.
    let mut moved = vec![(0.0f32, 0.0f32); n];
    // An inline element on several lines has one fragment, around all of its
    // lines; where its first line's piece begins inside that, from the
    // paragraph's pieces (`Node::ifc_inline_boxes`): (line, x, y) of the
    // first piece, and the corner of the pieces' union.
    let mut first_piece: Vec<Option<(usize, f32, f32)>> = vec![None; n];
    let mut pieces_corner = vec![(f32::MAX, f32::MAX); n];
    for i in 0..n {
        let Some(node) = dom.get_node(i) else {
            continue;
        };
        for &(id, (dx, dy)) in node.ifc_relative_offsets() {
            if let Some(m) = moved.get_mut(id) {
                m.0 += dx;
                m.1 += dy;
            }
        }
        for piece in node.ifc_inline_boxes().unwrap_or_default() {
            let (id, b) = (piece.node, piece.border_box);
            if id >= n {
                continue;
            }
            let corner = &mut pieces_corner[id];
            *corner = (corner.0.min(b.x), corner.1.min(b.y));
            if first_piece[id].is_none_or(|(line, x, _)| (piece.line, b.x) < (line, x)) {
                first_piece[id] = Some((piece.line, b.x, b.y));
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

    let mut placed = Placed {
        shown: vec![None; n],
        laid: vec![None; n],
        first: vec![None; n],
    };
    // The lowest fragment index seen of each node: its first fragment.
    let mut first_index = vec![u32::MAX; n];
    // Page coordinates to the continuous flow's: from the page area's corner,
    // and up by what the page begins above the flow's start. A box repeated
    // on every page (a fixed one) is placed in the page area, not the flow.
    let dx = -f64::from(area.x);
    let in_flow = f64::from(slice.content_origin_y) - f64::from(area.y);
    let in_page = -f64::from(area.y);
    let fragments: Vec<_> = dom
        .page_fragments(slice.page_index)
        .filter(|f| !matches!(f.kind(), FragmentKind::Text))
        .map(|f| {
            let fixed = matches!(f.repeat(), Some(RepeatKind::EveryPage));
            (f.node(), f.rect(), fixed, f.fragment_index())
        })
        .collect();
    let mut nodes: Vec<NodeId> = fragments.iter().map(|f| f.0).collect();
    nodes.sort_by_key(|id| id.0);
    nodes.dedup();
    let transforms = dom.page_paint_transforms(cascade, slice.page_index, nodes.iter().copied());
    for (node, rect, fixed, index) in fragments {
        let Ok(i) = usize::try_from(node.0) else {
            continue;
        };
        let Some(source) = dom.get_node(i) else {
            continue;
        };
        // `display: contents` makes no box, whatever Raikiri places for it.
        if source.generates_no_box() {
            continue;
        }
        let dy = if fixed { in_page } else { in_flow };
        let (mx, my) = moved[i];
        // A table's fragment is its grid's box; the element's box is the
        // wrapper around the grid and the captions.
        let rect = match source.table_grid_box() {
            Some(grid) => {
                let size = source.layout().size;
                PaintRect::new(rect.x - grid.x, rect.y - grid.y, size.width, size.height)
            }
            None => rect,
        };
        let rect = PaintRect::new(rect.x + mx, rect.y + my, rect.width, rect.height);
        // The size as the page's edges give it, bottom less top in the
        // page's `f32` coordinates, as `transformed` gives the shown box's:
        // on the page that begins ABOVE px up these are in whole 1/64 px, so
        // a height Raikiri's sums leave at 22.4999 is the 22.5 it is.
        let to_flow = |r: PaintRect| BoxRect {
            x: f64::from(r.x) + dx,
            y: f64::from(r.y) + dy,
            width: f64::from((r.x + r.width) - r.x),
            height: f64::from((r.y + r.height) - r.y),
        };
        let plain = to_flow(rect);
        // A box Raikiri puts nowhere a number can say has no box.
        if !is_finite(plain) {
            continue;
        }
        if reaches_farthest(plain) {
            return Err(LayoutRefusal::TooFar);
        }
        if !fixed && top.is_some_and(|top| plain.y <= top) {
            return Err(LayoutRefusal::AboveTop);
        }
        if index < first_index[i] {
            first_index[i] = index;
            // The first line's piece, where the fragment is around more.
            let (lx, ly) = first_piece[i].map_or((0.0, 0.0), |(_, x, y)| {
                (x - pieces_corner[i].0, y - pieces_corner[i].1)
            });
            placed.first[i] = Some((plain.x + f64::from(lx), plain.y + f64::from(ly)));
        }
        placed.laid[i] = Some(union(placed.laid[i], plain));
        let at = nodes.binary_search(&node).ok();
        let shown = match at.and_then(|k| transforms.get(k)) {
            // A singular transform: drawn nowhere, so no box from this.
            Some(None) => continue,
            Some(Some(t)) => transformed(rect, *t),
            None => rect,
        };
        let shown = to_flow(shown);
        if is_finite(shown) {
            placed.shown[i] = Some(union(placed.shown[i], shown));
        }
    }
    Ok(placed)
}

/// Whether a box of the page has a length or offset Raikiri clamps to
/// [`FARTHEST`] as it lays out. The clamped box is then often above the
/// page's top, where it has no fragment to show it, while what is placed
/// from it is placed wrong (`position: relative; top: -20000000px` and an
/// absolute child `top: 20000050px` puts the child at 0, where Chromium has
/// it at 50): such a page is not laid out. An element in a `display: none`
/// subtree has no box to place wrong (nor have its pseudo-elements), and a
/// `max-width` or `max-height` clamped to 10,000,000 px limits nothing the
/// clamp changes (the "no limit" idiom, `max-height: 99999999px`: the
/// `far-*` probes, Chromium's).
fn lays_out_too_far(dom: &raikiri_dom::Document, cascade: &raikiri_html::CascadeResult) -> bool {
    // Parents come before children in the arena.
    let mut hidden = vec![false; cascade.computed.len()];
    for (i, c) in cascade.computed.iter().enumerate() {
        let parent_hidden = dom
            .parent_of(i)
            .filter(|&p| p < i)
            .is_some_and(|p| hidden[p]);
        hidden[i] = parent_hidden || matches!(c.display, DisplayValue::None);
        if !hidden[i] && too_far(c) {
            return true;
        }
    }
    // A pseudo-element of an element with no box (one in a `display: none`
    // subtree) has none either, whatever its own display.
    cascade.pseudo.iter().any(|(&(owner, _), c)| {
        let owner_hidden = usize::try_from(owner.0)
            .ok()
            .and_then(|i| hidden.get(i).copied())
            .unwrap_or(false);
        !owner_hidden && !matches!(c.display, DisplayValue::None) && too_far(c)
    })
}

fn too_far(c: &ComputedValues) -> bool {
    use raikiri_style::{
        ComputedLengthPercentageOrAuto as Auto, ComputedLengthPercentageOrCalc as Calc,
    };
    let far = |v: f32| f64::from(v).abs() >= FARTHEST;
    let auto = |v: &Auto| matches!(v, Auto::Px(v) if far(*v));
    let sides = [
        &c.margin.top,
        &c.margin.right,
        &c.margin.bottom,
        &c.margin.left,
    ];
    let padding = [
        &c.padding.top,
        &c.padding.right,
        &c.padding.bottom,
        &c.padding.left,
    ];
    [
        &c.width,
        &c.height,
        &c.min_width,
        &c.min_height,
        &c.top,
        &c.right,
        &c.bottom,
        &c.left,
    ]
    .into_iter()
    .chain(sides)
    .any(auto)
        || padding
            .into_iter()
            .any(|v| matches!(v, Calc::Px(v) if far(*v)))
}

/// The weight of one level of content-sized box in [`too_costly`]'s estimate,
/// chosen conservatively (details with the orchestrator).
const CONTENT_SIZED_GROWTH: f64 = 3.0;

/// The estimate's budget. Ordinary pages cost their node count, at most
/// [`MAX_LAYOUT_NODES`].
const MAX_COST: f64 = 2_097_152.0;

/// A conservative estimate of intrinsic-sizing work, from the computed
/// styles, before the layout runs. A box sized by its content has an
/// intrinsic size keyword (`min-content`, `max-content`, `fit-content` or
/// `fit-content()`) in a width or height or their minimums or maximums; each
/// box weighs [`CONTENT_SIZED_GROWTH`] to the power of the number of such
/// boxes above it, boxes in a `display: none` subtree weigh nothing, and a
/// page over [`MAX_COST`] is refused. It over-counts on purpose: it counts
/// every display and both axes, `height` keywords and inline elements, where
/// a width keyword does nothing, included. It is a guard, not a bound: the
/// deadline is.
fn too_costly(dom: &raikiri_dom::Document, cascade: &raikiri_html::CascadeResult) -> bool {
    let content_sized = |c: &ComputedValues| {
        [
            &c.width,
            &c.min_width,
            &c.max_width,
            &c.height,
            &c.min_height,
            &c.max_height,
        ]
        .into_iter()
        .any(|v| v.is_content_keyword())
    };
    // Parents come before children in the arena. `None`: in a hidden subtree.
    let mut above: Vec<Option<i32>> = vec![Some(0); cascade.computed.len()];
    let mut cost = 0.0;
    for (i, c) in cascade.computed.iter().enumerate() {
        let parent = dom
            .parent_of(i)
            .filter(|&p| p < i)
            .map_or(Some(0), |p| above[p]);
        let Some(k) = parent.filter(|_| !matches!(c.display, DisplayValue::None)) else {
            above[i] = None;
            continue;
        };
        cost += CONTENT_SIZED_GROWTH.powi(k);
        if cost > MAX_COST {
            return true;
        }
        above[i] = Some(k.saturating_add(i32::from(content_sized(c))));
    }
    false
}

/// Whether `r` reaches as far from the page's corner as Raikiri lays out.
fn reaches_farthest(r: BoxRect) -> bool {
    r.x.min(r.y) <= -FARTHEST || (r.x + r.width).max(r.y + r.height) >= FARTHEST
}

/// The answers by page node id from what the fragments say: every element's
/// boxes, then its offsets.
fn read_boxes(
    dom: &raikiri_dom::Document,
    cascade: &raikiri_html::CascadeResult,
    placed: &Placed,
    back: &[usize],
    page_nodes: usize,
) -> RaikiriLaid {
    let n = dom.node_count();

    // offsetParent, offsetLeft and offsetTop as Chromium 153 answers them
    // (tests/data/chrome-layout/: offset-parents, root-offsets,
    // body-positioned, root-positioned, fixed-containers). The offset parent
    // is CSSOM View §7's: the nearest ancestor that is positioned, or a
    // containing block for positioned descendants, or `body`, or for a
    // static element a `td`, `th` or `table`; for a fixed element only an
    // ancestor that contains fixed boxes; none for the root, `body` or an
    // element with no box. Offsets are from the offset parent's padding
    // edge, transforms ignored, as in §7. **Where the offset parent is
    // `body` Chromium and §7 part, and this follows Chromium** (DECISIONS,
    // E3-R1): §7 says `body`'s padding edge, Chromium measures from the
    // border edge of the box the element's containing blocks lead to:
    // `body`'s if it is positioned, else the root element's, else (an
    // absolute element no box contains) the initial containing block, as
    // with no offset parent.
    let mut out = RaikiriLaid {
        boxes: vec![None; page_nodes],
        offsets: vec![None; page_nodes],
        sizes: vec![None; page_nodes],
    };
    let computed = |i: usize| cascade.computed.get(i);
    let position_is =
        |i: usize, want: fn(&PositionValue) -> bool| computed(i).is_some_and(|c| want(&c.position));
    // What Raikiri lays a fixed box out against (CSS Transforms 1 §5,
    // Filter Effects 1 §5, CSS Containment 2 §3.2 and §3.4, CSS Will Change 1
    // §3): the offset parent has to be the box the position came from.
    let contains_fixed = |i: usize| {
        computed(i).is_some_and(|c| {
            !c.transform.is_empty()
                || !c.filter.is_empty()
                || c.contain.contains_positioned_descendants()
                || [
                    WillChange::TRANSFORM,
                    WillChange::FILTER,
                    WillChange::CONTAIN,
                ]
                .into_iter()
                .any(|feature| c.will_change.contains(feature))
        })
    };
    let contains_positioned =
        |i: usize| position_is(i, |p| !matches!(p, PositionValue::Static)) || contains_fixed(i);
    let html = |i: usize, names: &[&str]| {
        dom.get_node(i).is_some_and(|node| {
            node.namespace_uri().is_none() && node.tag_name().is_some_and(|t| names.contains(&t))
        })
    };
    let parent_of = |i: usize| dom.parent_of(i).filter(|&p| p != 0);
    // The root element, and the body element: the root's first `body` child.
    let root = (1..n).find(|&i| {
        dom.parent_of(i) == Some(0) && dom.get_node(i).is_some_and(|x| x.tag_name().is_some())
    });
    let body = (1..n).find(|&i| root.is_some() && dom.parent_of(i) == root && html(i, &["body"]));
    let corner = |i: Option<usize>| i.and_then(|i| placed.first[i]).unwrap_or((0.0, 0.0));
    for i in 1..n {
        let Some(&live) = back.get(i) else { continue };
        if live == NONE || live >= page_nodes {
            continue;
        }
        out.boxes[live] = placed.shown[i];
        out.sizes[live] = placed.laid[i].map(|b| (b.width, b.height));
        let Some(first) = placed.first[i] else {
            continue;
        };
        if Some(i) == body {
            // §7 offsetTop step 1.
            out.offsets[live] = Some((0.0, 0.0));
            continue;
        }
        let fixed = position_is(i, |p| matches!(p, PositionValue::Fixed));
        let absolute = position_is(i, |p| matches!(p, PositionValue::Absolute));
        let static_self = position_is(i, |p| matches!(p, PositionValue::Static));
        let mut found = None;
        let mut parent = parent_of(i);
        while let Some(p) = parent {
            let is = if fixed {
                contains_fixed(p)
            } else {
                contains_positioned(p)
                    || Some(p) == body
                    || (static_self && html(p, &["td", "th", "table"]))
            };
            if is {
                found = Some(p);
                break;
            }
            parent = parent_of(p);
        }
        let from = match found {
            Some(p) if Some(p) != body => {
                let edge = placed.first[p].unwrap_or(first);
                let border = dom.get_node(p).map(|node| node.layout().border);
                let (bl, bt) = border.map_or((0.0, 0.0), |b| (f64::from(b.left), f64::from(b.top)));
                (edge.0 + bl, edge.1 + bt)
            }
            Some(_) if contains_positioned(body.unwrap_or(0)) => corner(body),
            Some(_) if absolute && !root.is_some_and(contains_positioned) => (0.0, 0.0),
            Some(_) => corner(root),
            None => (0.0, 0.0),
        };
        out.offsets[live] = Some((first.0 - from.0, first.1 - from.1));
    }
    out
}

/// One layout on a thread of its own, which the call waits for only until
/// the page's deadline (E3-01e's time bound; RAIKIRI.md section 10).
///
/// Raikiri can't be stopped inside its cascade or its layout pass, so the
/// call can't stop the work: it stops *waiting* for it. A layout the call
/// gave up on runs on to the end of the pass it is in, finds the deadline
/// passed at Raikiri's next abort check and ends, dropping its mirror on its
/// own thread. How long that takes is not bounded.
///
/// **The cap.** At most [`MAX_LAYOUT_THREADS`] layout threads live in the
/// process at once, running or given up on, so at most that many threads the
/// process can't stop, and as many mirrors. A slot is taken before the thread
/// starts, under one lock with the check, and the thread owns it: it is given
/// back when the thread ends, on every path (an answer, a layout given up on,
/// a panic, a thread that couldn't start). A call that finds every slot taken
/// waits for one, up to its deadline.
///
/// **What the cap costs.** While layouts given up on hold every slot, every
/// later Raikiri layout in the process ends with the time limit, whatever its
/// size and whoever's page it is: one page's answer can depend on pages that
/// ran before it in the process. That is a cost in determinism and in
/// isolation between pages, and one more reason the engine stays hidden.
///
/// **Fork.** The count is process-wide. A child forked while layouts run
/// inherits their slots but not their threads, so those slots are lost to it
/// for good, and a child forked while another thread holds the lock can
/// deadlock on its first layout. Unreachable today; a precondition for the
/// gem (reset the count in the child, or count per process generation).
///
/// Nothing is kept per thread between layouts: each runs on a new thread, and
/// its thread-local state goes with it.
mod bounded {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Condvar, Mutex, MutexGuard};
    use std::thread;
    use std::time::Duration;

    use surimono_core::Deadline;

    use super::time_limit;
    use crate::DomError;
    use crate::layout::EngineError;

    /// Most layout threads, running or given up on, in the process. Each
    /// holds at most one mirror (`MAX_LAYOUT_NODES`).
    pub(crate) const MAX_LAYOUT_THREADS: usize = 4;

    /// How often a waiting call looks at its deadline.
    const POLL: Duration = Duration::from_millis(1);

    /// The layout thread's stack. Raikiri grows its own on the heap for deep
    /// boxes (`stacker`), so this is what our code and its shallow frames
    /// need, with room to spare; it is reserved, not resident.
    const STACK: usize = 8 << 20;

    static LIVE: Mutex<usize> = Mutex::new(0);
    static FREED: Condvar = Condvar::new();

    fn live_lock() -> MutexGuard<'static, usize> {
        // Nothing panics while holding it; a poisoned count is still a count.
        LIVE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Layout threads alive now, running or given up on.
    pub(crate) fn live() -> usize {
        *live_lock()
    }

    /// One of the [`MAX_LAYOUT_THREADS`] slots, given back when dropped.
    struct Slot;

    impl Drop for Slot {
        fn drop(&mut self) {
            let mut n = live_lock();
            *n = n.saturating_sub(1);
            drop(n);
            FREED.notify_all();
        }
    }

    /// Takes a slot, waiting for one up to `deadline`. With no deadline (a
    /// `Document` driven directly, never from the bindings) the wait has no
    /// bound.
    fn take_slot(deadline: Option<&Deadline>) -> Result<Slot, EngineError> {
        let mut n = live_lock();
        while *n >= MAX_LAYOUT_THREADS {
            if deadline.is_some_and(Deadline::passed) {
                return Err(time_limit().into());
            }
            n = match FREED.wait_timeout(n, POLL) {
                Ok((n, _)) => n,
                Err(e) => e.into_inner().0,
            };
        }
        *n += 1;
        Ok(Slot)
    }

    fn fault(message: &str) -> EngineError {
        EngineError::Fault(DomError::LayoutFailed {
            message: message.to_owned(),
        })
    }

    /// Runs `work` on a new thread and gives its result, or the time limit
    /// once `deadline` has passed while it still runs.
    pub(crate) fn run<T: Send + 'static>(
        deadline: Option<&Deadline>,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, EngineError> {
        let passed = || deadline.is_some_and(Deadline::passed);
        let slot = take_slot(deadline)?;
        let (tx, rx) = mpsc::sync_channel::<T>(1);
        // If the thread can't start, the closure is dropped here, and the
        // slot with it.
        let handle = thread::Builder::new()
            .name("surimono-layout".to_owned())
            .stack_size(STACK)
            .spawn(move || {
                // The slot goes back when this closure ends, however it ends.
                let _slot = slot;
                // `work` catches its own panics; one that got out all the
                // same leaves the channel closed, which the call reads as a
                // fault.
                if let Ok(out) = panic::catch_unwind(AssertUnwindSafe(work)) {
                    // If the call gave up, the result is dropped here (or,
                    // sent just as it gave up, with the channel).
                    let _ = tx.send(out);
                }
            })
            .map_err(|_| fault("the layout thread could not be started"))?;
        loop {
            let got = if deadline.is_some() {
                rx.recv_timeout(POLL)
            } else {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match got {
                Ok(out) => {
                    // It has sent: what is left is the thread's own exit,
                    // which gives the slot back.
                    let _ = handle.join();
                    return Ok(out);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = handle.join();
                    return Err(fault("the layout thread ended without an answer"));
                }
                Err(RecvTimeoutError::Timeout) if passed() => {
                    if let Ok(out) = rx.try_recv() {
                        let _ = handle.join();
                        return Ok(out);
                    }
                    // The thread runs on, detached, and gives its slot back
                    // when it ends.
                    return Err(time_limit().into());
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
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
        assert!(matches!(
            guarded(|| Err(Stop::Refused(LayoutRefusal::EngineBound))),
            Err(EngineError::Refused(LayoutRefusal::EngineBound))
        ));
        assert!(matches!(
            guarded(|| Err(Stop::TimeLimit)),
            Err(EngineError::Binding(BindingError::TypeError(_)))
        ));
    }

    #[test]
    fn a_layout_thread_gives_its_answer_or_a_typed_fault() {
        let got = bounded::run(None, || 6 * 7);
        assert!(matches!(got, Ok(42)), "{got:?}");
        // A panic the work didn't catch ends the call with a fault, and the
        // thread is gone with its slot.
        let got = bounded::run(None, || -> u8 { panic!("outside guarded") });
        assert!(
            matches!(&got, Err(EngineError::Fault(DomError::LayoutFailed { message }))
                if message == "the layout thread ended without an answer"),
            "{got:?}"
        );
        assert_eq!(bounded::live(), 0);
        // What the work owns is dropped on its thread.
        let owned = Arc::new(());
        let theirs = Arc::clone(&owned);
        let got = bounded::run(None, move || Arc::strong_count(&theirs));
        assert!(matches!(got, Ok(2)), "{got:?}");
        assert_eq!(Arc::strong_count(&owned), 1);
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
    fn a_rectangle_no_number_can_say_is_not_finite() {
        let r = |x: f64, width: f64| BoxRect {
            x,
            y: 0.0,
            width,
            height: 1.0,
        };
        assert!(is_finite(r(-1e30, 1e300)));
        assert!(!is_finite(r(f64::NAN, 1.0)));
        assert!(!is_finite(r(0.0, f64::INFINITY)));
        // What a transform of an `f32`'s largest numbers gives.
        let t = transformed(
            PaintRect::new(0.0, 0.0, 1e30, 1.0),
            PaintTransform::new(1e30, 0.0, 0.0, 1.0, 0.0, 0.0),
        );
        assert!(!t.width.is_finite());
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
