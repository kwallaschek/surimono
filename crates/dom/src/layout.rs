//! Layout queries answered from a real layout (E3-00 prototype, internal).
//!
//! This file is the seam between the bindings and whatever lays the page
//! out. It knows when a layout is needed and when one is stale, and asks an
//! engine for a [`Laid`] page through [`Engine`]; it names no type of any
//! layout engine but the font context and the Raikiri setup in
//! [`LayoutSetup`]. The engines are Blitz, in `layout_blitz.rs`, the layout
//! fulgur 0.40 prints with, and with the `raikiri` feature Raikiri, in
//! `layout_raikiri.rs`, fulgur-raikiri's (E3-R1).
//!
//! `DESIGN.md`, "Layout queries (phase 3)", has the reasons and the costs.

use std::rc::Rc;

use blitz_dom::FontContext;
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};

use crate::DomError;
use crate::document::{Document, LayoutPolicy};
use crate::layout_blitz::Blitz;
#[cfg(feature = "raikiri")]
use crate::layout_raikiri::Raikiri;

/// What a real layout is run against. Internal: the phase-3 public API waits
/// for checkpoint 3, and the facade doesn't hand this out.
#[doc(hidden)]
#[derive(Clone)]
pub struct LayoutSetup {
    /// The viewport's width in CSS pixels: for print, the page's width less
    /// its side margins at 96 dpi.
    pub width: u32,
    /// The viewport's height in CSS pixels.
    pub height: u32,
    /// Which engine lays the page out, with its fonts.
    pub engine: LayoutEngine,
}

/// The engine behind [`LayoutSetup`], with what it measures text with.
/// Internal, like the setup.
#[doc(hidden)]
#[derive(Clone)]
pub enum LayoutEngine {
    /// Blitz, the E3-00 prototype, with the fonts text is measured with:
    /// build the context with `system_fonts: false` from the same font
    /// bytes the printer gets.
    Blitz(Box<FontContext>),
    /// Raikiri (E3-R1), with its fonts and page.
    #[cfg(feature = "raikiri")]
    Raikiri(crate::layout_raikiri::RaikiriSetup),
}

impl std::fmt::Debug for LayoutEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blitz(_) => f.write_str("Blitz"),
            #[cfg(feature = "raikiri")]
            Self::Raikiri(setup) => f.debug_tuple("Raikiri").field(setup).finish(),
        }
    }
}

impl LayoutSetup {
    /// Blitz's fonts, if it is the engine.
    pub(crate) fn blitz_fonts(&self) -> Option<&FontContext> {
        match &self.engine {
            LayoutEngine::Blitz(fonts) => Some(fonts.as_ref()),
            #[cfg(feature = "raikiri")]
            LayoutEngine::Raikiri(_) => None,
        }
    }

    /// The Raikiri engine's setup, if it is the engine.
    #[cfg(feature = "raikiri")]
    pub(crate) fn raikiri(&self) -> Option<&crate::layout_raikiri::RaikiriSetup> {
        match &self.engine {
            LayoutEngine::Raikiri(setup) => Some(setup),
            LayoutEngine::Blitz(_) => None,
        }
    }
}

impl std::fmt::Debug for LayoutSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutSetup")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

/// A rectangle in CSS pixels, in viewport coordinates. Nothing scrolls, so
/// those are the document's too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A layout engine, as the bindings see one: it turns the page as it is now
/// into a [`Laid`] page. Everything it needs comes in through the two
/// arguments: the tree, the sheets and the style attributes from the page,
/// the viewport and the fonts from the setup.
pub(crate) trait Engine {
    /// Lays `page` out. `Ok(None)` for a page with nothing to lay out (no
    /// root element, no setup for this engine). [`EngineError::Refused`]
    /// when the engine refuses the page (too much for one layout): queries
    /// then answer as with no box, and the page counts the refusal. Any
    /// other error ends the script's call; the engine looks at the page's
    /// deadline (`Document::poll`) wherever its own work can be cut.
    fn lay_out(page: &Document, setup: &LayoutSetup) -> EngineResult;
}

/// What [`Engine::lay_out`] gives.
pub(crate) type EngineResult = Result<Option<Box<dyn Laid>>, EngineError>;

/// Why an engine gave no layout.
#[derive(Debug)]
pub(crate) enum EngineError {
    /// The page is over one of the engine's bounds and is not laid out:
    /// queries answer as with no box, and the reason is counted
    /// ([`LayoutRefusals`]) so that a caller can tell a refused page from
    /// boxes that are really empty.
    Refused(LayoutRefusal),
    /// Thrown into the script, as a native's error is: the time limit.
    Binding(BindingError),
    /// Fatal to the call: the engine failed on this page. Only the Raikiri
    /// engine reports one.
    #[cfg_attr(not(feature = "raikiri"), allow(dead_code))]
    Fault(DomError),
}

/// Why a page was not laid out, as counted in [`LayoutRefusals`]. Internal,
/// like the layout itself.
#[doc(hidden)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutRefusal {
    /// More elements and text nodes than one layout takes.
    Nodes,
    /// More text than one layout takes.
    Text,
    /// More attribute text than one layout takes, or a style attribute
    /// over the bounds a `style` read has.
    Attributes,
    /// More style sheet text than one layout takes, or style (sheets and
    /// style attributes together) over the shape bounds (`sheet_screen.rs`).
    Sheets,
    /// A box or a length as far out as the engine lays out (Raikiri's
    /// 10,000,000 px), where its numbers aren't the page's.
    TooFar,
    /// A box cut at the top of the page the engine measures on.
    AboveTop,
    /// One of the engine's own bounds: pages, fragments, rules, a
    /// paragraph's size, the depth of boxes.
    EngineBound,
    /// A page whose shape would make one layout take longer than a layout
    /// may (`layout_raikiri.rs`, the cost estimate).
    Cost,
}

impl LayoutRefusal {
    const ALL: [Self; 8] = [
        Self::Nodes,
        Self::Text,
        Self::Attributes,
        Self::Sheets,
        Self::TooFar,
        Self::AboveTop,
        Self::EngineBound,
        Self::Cost,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|&r| r == self).unwrap_or(0)
    }
}

/// How many layouts of a document were refused, by reason. Internal.
#[doc(hidden)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutRefusals {
    counts: [u64; LayoutRefusal::ALL.len()],
}

impl LayoutRefusals {
    /// Refusals for `reason`.
    pub fn count(&self, reason: LayoutRefusal) -> u64 {
        self.counts[reason.index()]
    }

    /// Refusals for any reason.
    pub fn total(&self) -> u64 {
        self.counts.iter().sum()
    }

    pub(crate) fn add(&mut self, reason: LayoutRefusal) {
        let c = &mut self.counts[reason.index()];
        *c = c.saturating_add(1);
    }
}

impl From<BindingError> for EngineError {
    fn from(e: BindingError) -> Self {
        Self::Binding(e)
    }
}

/// A page that has been laid out: what queries are answered from until the
/// page changes. Nodes are the page's node ids, never the engine's.
///
/// The prototype has one question. E3-01 and E3-02 add the others here and
/// nowhere else: client rectangles of inline content, padding, border and
/// content sizes, a computed value by property name, a media query, a hit
/// test.
pub(crate) trait Laid {
    /// The border box of page node `node`, as laid out, before any pixel
    /// snapping. `None` for a node with no box of its own.
    fn border_box(&self, node: usize) -> Option<BoxRect>;

    /// `offsetLeft` and `offsetTop` of page node `node`, before rounding
    /// (CSSOM View §7, but from the edge Chromium measures from where the
    /// offset parent is `body`: `layout_raikiri.rs`).
    /// `None` where the engine doesn't answer them: zeros. Blitz doesn't.
    fn offset_position(&self, _node: usize) -> Option<(f64, f64)> {
        None
    }

    /// The size `offsetWidth` and `offsetHeight` round (CSSOM View §7): the
    /// border box as laid out, before any transform. `None` where the engine
    /// has no size apart from [`Laid::border_box`]'s. Blitz has none.
    fn offset_size(&self, _node: usize) -> Option<(f64, f64)> {
        None
    }
}

/// A length as the whole number of pixels `offsetLeft` and its like answer,
/// as Chromium 153 has them: first in whole 1/64 px, as it lays out, with
/// the fraction cut off toward zero (`left: -0.51px` is -0.5 px there, and
/// `width: 4.49px` 4.484375: the `round-trunc` probe), then the nearest
/// whole number, and of two equally near the greater (`fractions`: -2.5 is
/// -2, 2.5 is 3). CSSOM View §7 doesn't say how. The engines lay out in
/// `f32`, whose last bits are noise: a hair away from zero (3/1000 of a
/// 1/64 px) is added before the cut, so that a height of 22.4999985 is 22.5
/// again, and `left: 2.4999px` still 2.484375 as in Chromium.
fn whole(v: f64) -> f64 {
    let units = v * 64.0;
    let hair: f64 = 3e-3;
    let snapped = (units + hair.copysign(units)).trunc() / 64.0;
    (snapped + 0.5).floor()
}

/// The last layout and what it was made from.
pub(crate) struct Current {
    /// The page's `(version, layout_epoch)` then.
    stamp: (u64, u64),
    /// `None` if the engine refused the page or had nothing to lay out.
    laid: Option<Box<dyn Laid>>,
}

impl Document {
    /// One number of an element's box, for the glue: 0 to 3 are
    /// `getBoundingClientRect`'s x, y, width and height, 4 and 5
    /// `offsetWidth` and `offsetHeight`, 6 and 7 `offsetLeft` and
    /// `offsetTop`. Zero under any policy but [`LayoutPolicy::Real`], and for
    /// an element with no box.
    pub(crate) fn layout_number(&mut self, this: ObjectId, which: u32) -> BindingResult<f64> {
        let id = self.node_of(this)?;
        if self.layout != LayoutPolicy::Real {
            return Ok(0.0);
        }
        self.ensure_layout()?;
        let laid = self.current_layout.as_ref().and_then(|c| c.laid.as_ref());
        if which == 6 || which == 7 {
            let at = laid.and_then(|l| l.offset_position(id)).unwrap_or_default();
            // Integers, as offsetWidth is.
            return Ok(whole(if which == 6 { at.0 } else { at.1 }));
        }
        if which == 4 || which == 5 {
            // CSSOM View §7: the border edge size as an integer, which no
            // transform changes.
            let size = laid
                .and_then(|l| {
                    l.offset_size(id)
                        .or_else(|| l.border_box(id).map(|b| (b.width, b.height)))
                })
                .unwrap_or_default();
            return Ok(whole(if which == 4 { size.0 } else { size.1 }));
        }
        let Some(b) = laid.and_then(|l| l.border_box(id)) else {
            return Ok(0.0);
        };
        Ok(match which {
            0 => b.x,
            1 => b.y,
            2 => b.width,
            3 => b.height,
            _ => 0.0,
        })
    }

    /// Layouts run on this document so far.
    pub(crate) fn layouts_run(&self) -> u64 {
        self.layouts_run
    }

    /// Layouts of this document refused so far, by reason.
    pub(crate) fn layout_refusals(&self) -> &LayoutRefusals {
        &self.layout_refusals
    }

    /// Lays the page out unless the last layout is still current. A change
    /// to the tree, an attribute, text or a sheet makes it stale: one flag
    /// for the whole document.
    fn ensure_layout(&mut self) -> BindingResult<()> {
        let stamp = (self.version, self.layout_epoch);
        if self
            .current_layout
            .as_ref()
            .is_some_and(|c| c.stamp == stamp)
        {
            return Ok(());
        }
        // The old one goes first, so that only one is alive at a time.
        self.current_layout = None;
        let Some(setup) = self.layout_setup.clone() else {
            return Ok(());
        };
        let setup: &Rc<LayoutSetup> = &setup;
        let outcome = match &setup.engine {
            LayoutEngine::Blitz(_) => Blitz::lay_out(self, setup),
            #[cfg(feature = "raikiri")]
            LayoutEngine::Raikiri(_) => Raikiri::lay_out(self, setup),
        };
        let laid = match outcome {
            Ok(laid) => laid,
            Err(EngineError::Refused(reason)) => {
                self.layout_refusals.add(reason);
                None
            }
            Err(EngineError::Binding(e)) => return Err(e),
            Err(EngineError::Fault(e)) => return Err(self.fail(e)),
        };
        if laid.is_some() {
            self.layouts_run += 1;
        }
        self.current_layout = Some(Current { stamp, laid });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::whole;

    #[test]
    fn whole_numbers_are_chromiums() {
        // tests/data/chrome-layout: `fractions` and `round-trunc`, Chromium's.
        for (v, want) in [
            (2.5, 3.0),
            (-2.5, -2.0),
            (1.5, 2.0),
            (-1.5, -1.0),
            (0.49, 0.0),
            (-0.51, 0.0),
            (-1.51, -1.0),
            (-0.52, -1.0),
            (-1.4999, -1.0),
            (4.49, 4.0),
            (4.51, 5.0),
            (-0.008, 0.0),
            (1.992, 2.0),
            // f32 noise under a half.
            (22.499_998_5, 23.0),
            (2.4999, 2.0),
            (-0.515, 0.0),
            (-0.499_999_9, 0.0),
            (9_999_990.0, 9_999_990.0),
            (f64::from(9_999_990.0_f32) + 0.4, 9_999_990.0),
        ] {
            assert_eq!(whole(v), want, "{v}");
        }
    }
}
