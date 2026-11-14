//! Blitz as the layout engine behind `layout.rs` (E3-00 prototype).
//!
//! The page's document is a tree store that keeps blitz-dom's document
//! behaviour switched off (see `DESIGN.md`, "Staying out of blitz-dom's
//! document behaviour"). Layout therefore runs on a *mirror*: a second
//! `BaseDocument` built from the page's tree, laid out once, and thrown away
//! when the page changes. Blitz's layout boxes (anonymous blocks,
//! pseudo-elements, markers) live and die in the mirror and never count
//! against the page's limits or meet its handles.

use blitz_dom::{Attribute, BaseDocument, DocumentConfig, ElementData, NodeData, ns};
use blitz_traits::shell::{ColorScheme, Viewport};
use style::context::QuirksMode;
use style::media_queries::MediaList;
use style::properties::{PropertyDeclarationBlock, parse_style_attribute};
use style::servo_arc::Arc as ServoArc;
use style::stylesheets::{
    AllowImportRules, CssRuleType, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
};
use style::values::computed::Display;
use style::values::specified::box_::{DisplayInside, DisplayOutside};
use surimono_webidl::runtime::{BindingError, BindingResult};

use crate::document::{Document, Kind};
use crate::layout::{BoxRect, Engine, EngineResult, Laid, LayoutSetup};
use crate::sheet_screen::{Refused, Screen};
use crate::style_attr;

/// No mirror node: a comment, a node outside the page's tree.
const NONE: usize = usize::MAX;

/// Most style-sheet text one layout takes, in all. With [`MAX_TEXT_BYTES`],
/// [`MAX_ATTRIBUTE_BYTES`] and the shape bounds of `sheet_screen.rs` it is
/// what one layout is bounded by besides the page's node limit: a layout
/// can't be stopped once started. Past any of them the layout isn't run and
/// queries answer as with no box.
pub(crate) const MAX_SHEET_BYTES: usize = 1 << 20;

/// Most text one layout takes, in all: the text that can be rendered, so
/// not a script's source or a title. The page's own text limit (256 MiB) is
/// no bound for a layout.
pub(crate) const MAX_TEXT_BYTES: usize = 256 << 10;

/// Longest attribute value copied whole. Layout reads short ones (sizes,
/// names, the words selectors match); a longer one is in practice a `data:`
/// URL or path data, and the copy has its first bytes. `class` and `id` are
/// always whole.
const MAX_ATTRIBUTE_VALUE: usize = 2 << 10;

/// Most attribute bytes one layout copies, in all.
pub(crate) const MAX_ATTRIBUTE_BYTES: usize = 8 << 20;

/// The page as Blitz laid it out.
struct Mirror {
    doc: BaseDocument,
    /// Page node id to mirror node id, [`NONE`] for none.
    map: Vec<usize>,
}

/// The engine: see [`Engine`].
pub(crate) struct Blitz;

impl Engine for Blitz {
    fn lay_out(page: &Document, setup: &LayoutSetup) -> EngineResult {
        Ok(build_mirror(page, setup)?.map(|m| Box::new(m) as Box<dyn Laid>))
    }
}

fn time_limit() -> BindingError {
    BindingError::TypeError("surimono: the time limit has passed".to_owned())
}

impl Laid for Mirror {
    fn border_box(&self, node: usize) -> Option<BoxRect> {
        border_box(self, node)
    }
}

/// Copies the page's tree into a new Blitz document, gives it the page's
/// style sheets and style attributes, and lays it out. The copy looks at
/// the deadline as it goes; the layout itself can't be stopped once
/// started, so what goes into it is bounded first: the page's node limit,
/// [`MAX_TEXT_BYTES`], [`MAX_ATTRIBUTE_BYTES`], [`MAX_SHEET_BYTES`] and the
/// sheets' shape (`sheet_screen.rs`). `None` for a page over any of them.
fn build_mirror(page: &Document, setup: &LayoutSetup) -> BindingResult<Option<Mirror>> {
    let Some(fonts) = setup.blitz_fonts() else {
        return Ok(None);
    };
    let mut doc = BaseDocument::new(DocumentConfig {
        viewport: Some(Viewport::new(
            setup.width,
            setup.height,
            1.0,
            ColorScheme::Light,
        )),
        font_ctx: Some(fonts.clone()),
        ..DocumentConfig::default()
    });
    let lock = doc.guard().clone();
    let quirks = page.page_quirks();
    let mut map = Vec::new();
    let mut sheets: Vec<(usize, usize)> = Vec::new();
    let mut text_bytes = 0usize;
    let mut attribute_bytes = 0usize;
    let mut screen = Screen::default();
    let mirror_root = doc.root_node().id;
    // (page node, its mirror parent), popped in document order, so mirror
    // ids rise in document order: blitz-dom orders sheets by node id.
    let mut stack: Vec<(usize, usize)> = page
        .children(page.root())
        .iter()
        .rev()
        .map(|&c| (c, mirror_root))
        .collect();
    while let Some((live, parent)) = stack.pop() {
        page.poll()?;
        let data = match page.kind(live) {
            Some(Kind::Element) => {
                let Some(e) = page.node(live).element_data() else {
                    continue;
                };
                let is_list = e.name.local.as_ref() == "ol";
                let mut attrs = Vec::with_capacity(e.attrs().len());
                let mut style = None;
                for a in e.attrs() {
                    let local = a.name.local.as_ref();
                    let value = if a.name.ns == ns!() && local == "style" {
                        // Its declarations go in as a block; the attribute
                        // stays, empty, for `[style]` to match.
                        style = Some(a.value.as_str());
                        ""
                    } else if is_list && local == "start" && !counts_from(&a.value) {
                        // The measuring copy numbers a list from 1 unless
                        // `start` is a positive number in HTML's range.
                        // Marker text isn't queried, and a marker sits
                        // outside its item's principal box.
                        continue;
                    } else {
                        copied(local, &a.value)
                    };
                    attribute_bytes = attribute_bytes.saturating_add(value.len());
                    if attribute_bytes > MAX_ATTRIBUTE_BYTES {
                        return Ok(None);
                    }
                    attrs.push(Attribute {
                        name: a.name.clone(),
                        value: value.to_owned(),
                    });
                }
                let mut data = ElementData::new(e.name.clone(), attrs);
                if let Some(style) = style {
                    let block = match declaration_block(page, style, quirks, &mut screen) {
                        Ok(block) => block,
                        Err(style_attr::Stop::TimeLimit) => return Err(time_limit()),
                        // Refused where it was written; not laid out if it
                        // is here all the same.
                        Err(style_attr::Stop::TooLarge) => return Ok(None),
                    };
                    data.style_attribute = Some(ServoArc::new(lock.wrap(block)));
                }
                NodeData::Element(data)
            }
            Some(Kind::Text) => {
                let text = page.char_data(live);
                text_bytes = text_bytes.saturating_add(text.len());
                if text_bytes > MAX_TEXT_BYTES {
                    return Ok(None);
                }
                NodeData::Text(blitz_dom::TextNodeData::new(text.to_owned()))
            }
            // Comments, doctypes and processing instructions have no box.
            _ => continue,
        };
        let is_sheet = matches!(&data, NodeData::Element(e)
            if e.name.local.as_ref() == "style" && (e.name.ns == ns!(html) || e.name.ns == ns!(svg)));
        // Children that are never rendered have no boxes, and their text is
        // not text a layout takes: an image's and a canvas's (HTML §4.8.3,
        // §4.12.5: fallback content), a script's source, a title, and what
        // `noscript` holds while scripts run (§4.12.2).
        let is_replaced = matches!(&data, NodeData::Element(e)
            if (e.name.ns == ns!(html)
                && matches!(
                    e.name.local.as_ref(),
                    "img" | "canvas" | "script" | "title" | "noscript" | "template"
                ))
                || (e.name.ns == ns!(svg) && e.name.local.as_ref() == "script"));
        let id = doc.create_node(data);
        if let Some(node) = doc.get_node_mut(id) {
            node.parent = Some(parent);
        }
        if let Some(p) = doc.get_node_mut(parent) {
            p.children.push(id);
        }
        if live >= map.len() {
            map.resize(live + 1, NONE);
        }
        map[live] = id;
        if is_sheet {
            // Its text goes to stylo below, not into the tree.
            sheets.push((live, id));
        } else if !is_replaced {
            stack.extend(page.children(live).iter().rev().map(|&c| (c, id)));
        }
    }

    let mut sheet_bytes = 0usize;
    let texts: Vec<(usize, String)> = sheets
        .into_iter()
        .map(|(live, id)| {
            let text = page
                .edited_sheet_text(live)
                .unwrap_or_else(|| page.style_text(live));
            sheet_bytes = sheet_bytes.saturating_add(text.len());
            (id, text)
        })
        .collect();
    if sheet_bytes > MAX_SHEET_BYTES {
        return Ok(None);
    }
    // The shape of every sheet, before the style engine sees any of them.
    for (_, text) in &texts {
        screen.sheet(text, page.deadline.as_ref());
    }
    match screen.finish() {
        Ok(()) => {}
        Err(Refused::TooMuch) => return Ok(None),
        Err(Refused::TimeLimit) => return Err(time_limit()),
    }
    let laid_out = doc.try_root_element().is_some();
    if laid_out {
        for (id, text) in texts {
            if page.deadline.as_ref().is_some_and(|d| d.passed()) {
                return Err(time_limit());
            }
            // Made here, with no loader and `@import` off: nothing is
            // fetched for layout.
            let sheet = Stylesheet::from_str(
                &text,
                sheet_base(),
                Origin::Author,
                ServoArc::new(lock.wrap(MediaList::empty())),
                lock.clone(),
                None,
                None,
                QuirksMode::NoQuirks,
                AllowImportRules::No,
            );
            doc.add_stylesheet_for_node(DocumentStyleSheet(ServoArc::new(sheet)), id);
        }
        if page.deadline.as_ref().is_some_and(|d| d.passed()) {
            return Err(time_limit());
        }
        // No animation runs here: the time is always zero.
        doc.resolve(0.0);
    }
    Ok(laid_out.then_some(Mirror { doc, map }))
}

/// A style attribute as stylo's declaration block, parsed a declaration
/// at a time with the deadline looked at in between, as a `style` read
/// does. What [`style_attr::fits`] let in bounds the work; `screen` hears
/// which custom properties name which.
fn declaration_block(
    page: &Document,
    text: &str,
    quirks: QuirksMode,
    screen: &mut Screen,
) -> Result<PropertyDeclarationBlock, style_attr::Stop> {
    let declarations = style_attr::parse(text, quirks, page.deadline.as_ref())?;
    let mut block = PropertyDeclarationBlock::new();
    for d in declarations {
        if page.deadline.as_ref().is_some_and(|d| d.passed()) {
            return Err(style_attr::Stop::TimeLimit);
        }
        if d.value.len() > style_attr::MAX_CHECKED_VALUE {
            // In practice a `data:` image, which changes no box.
            continue;
        }
        screen.attribute_declaration(&d.name, &d.value);
        let important = if d.important { " !important" } else { "" };
        let one = parse_style_attribute(
            &format!("{}: {}{important}", d.name, d.value),
            style_attr::base(),
            None,
            quirks,
            CssRuleType::Style,
        );
        for (declaration, importance) in one.declaration_importance_iter() {
            block.push(declaration.clone(), importance);
        }
    }
    Ok(block)
}

/// Whether an `ol`'s `start` is a number the copy counts from.
fn counts_from(start: &str) -> bool {
    start
        .parse::<usize>()
        .is_ok_and(|n| (1..=i32::MAX as usize).contains(&n))
}

/// What of attribute `local`'s `value` the copy holds: all of a short one,
/// of `class` and of `id`, and the start of any other.
pub(crate) fn copied<'v>(local: &str, value: &'v str) -> &'v str {
    if value.len() <= MAX_ATTRIBUTE_VALUE || matches!(local, "class" | "id") {
        return value;
    }
    let mut end = MAX_ATTRIBUTE_VALUE;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// The border box of page node `id` in viewport coordinates (nothing
/// scrolls): x, y, width, height in CSS pixels, as laid out, before
/// Taffy's pixel snapping. `None` for a node with no box of its own:
/// outside the page's tree, `display: none` or under one, `display:
/// contents`, and (a gap, see DESIGN.md) a non-replaced inline element.
fn border_box(mirror: &Mirror, id: usize) -> Option<BoxRect> {
    let at = *mirror.map.get(id).filter(|&&m| m != NONE)?;
    let root = mirror.doc.try_root_element()?.id;
    let node = mirror.doc.get_node(at)?;
    // An inline element in running text is not a box to Blitz: its glyphs
    // are runs of the line's layout. Until those are read (DESIGN.md), it
    // answers as with no box. The names are the ones blitz-dom makes inline
    // boxes for (`find_inline_layout_embedded_boxes`).
    let display = node.primary_styles()?.clone_display();
    let atomic = node.element_data().is_some_and(|e| {
        matches!(
            e.name.local.as_ref(),
            "img" | "svg" | "input" | "textarea" | "button"
        )
    });
    if display.outside() == DisplayOutside::Inline
        && display.inside() == DisplayInside::Flow
        && !atomic
    {
        return None;
    }
    let size = node.unrounded_layout.size;
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let mut node = node;
    loop {
        if node
            .primary_styles()
            .is_none_or(|s| s.clone_display() == Display::None)
        {
            return None;
        }
        x += f64::from(node.unrounded_layout.location.x);
        y += f64::from(node.unrounded_layout.location.y);
        if node.id == root {
            break;
        }
        // Only boxes have a layout parent; the links end at the root
        // element, so the walk is as long as the box is deep.
        node = mirror.doc.get_node(node.layout_parent.get()?)?;
    }
    Some(BoxRect {
        x,
        y,
        width: f64::from(size.width),
        height: f64::from(size.height),
    })
}

/// The base `url()`s in the page's sheets parse against. Nothing is fetched.
fn sheet_base() -> UrlExtraData {
    style_attr::base().clone()
}
