//! Text measurement for renderers, behind `ctx.measureText(text, cssFont)`.
//!
//! Documents measure with [`surimono_gfx::TextMeasurer`]: the caller's
//! fonts, shaped as fulgur prints them. A runner without fonts fails any
//! island that measures text with [`IslandError::NoFonts`](crate::IslandError::NoFonts).
//! The font-free estimate here is for surimono's own tests and tools only.

use std::fmt;
#[cfg(test)]
use std::sync::Arc;

use surimono_gfx::{GfxError, TextMeasurer, TextMetrics};

/// How a runner's workers answer `ctx.measureText`.
#[derive(Clone)]
pub(crate) enum Measure {
    /// The caller's fonts.
    Fonts(TextMeasurer),
    /// No fonts were given: measuring fails with `NoFonts`.
    NoFonts,
    /// The font-free estimate: 0.6 em per character, 1 em for wide (CJK)
    /// characters, ascent 0.8 em, descent 0.2 em. Tests and tools only.
    #[cfg(any(test, feature = "test-estimate"))]
    Estimate,
    /// Any function, for the runner's own tests (errors, panics).
    #[cfg(test)]
    Custom(Arc<CustomMeasure>),
}

/// A measuring function for tests.
#[cfg(test)]
pub(crate) type CustomMeasure = dyn Fn(&str, &str) -> Result<TextMetrics, String> + Send + Sync;

/// Why `measureText` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MeasureError {
    /// There are no fonts to measure with.
    NoFonts,
    /// Anything else, e.g. text over the measurer's size limit.
    Other(String),
}

impl Measure {
    pub(crate) fn new(measurer: Option<TextMeasurer>) -> Self {
        measurer.map_or(Self::NoFonts, Self::Fonts)
    }

    /// A measurer for one worker thread: the same fonts (shared, not
    /// copied), with measuring state of its own. A shared measurer creates
    /// a workspace for each call that overlaps another and keeps it, so how
    /// many it holds depends on timing; one per worker is created by the
    /// worker's first measurement and stays the only one.
    pub(crate) fn for_worker(&self) -> Self {
        match self {
            Self::Fonts(m) => Self::Fonts(TextMeasurer::new(m.fonts().clone())),
            other => other.clone(),
        }
    }

    /// Measures `text` in the CSS `font` shorthand `css_font`.
    pub(crate) fn measure(&self, text: &str, css_font: &str) -> Result<TextMetrics, MeasureError> {
        match self {
            // No fonts comes first, whatever else is wrong with the text.
            Self::Fonts(m) if m.fonts().face_count() == 0 => Err(MeasureError::NoFonts),
            Self::Fonts(m) => m.measure(text, css_font).map_err(|e| match e {
                GfxError::NoFonts => MeasureError::NoFonts,
                other => MeasureError::Other(other.to_string()),
            }),
            Self::NoFonts => Err(MeasureError::NoFonts),
            #[cfg(any(test, feature = "test-estimate"))]
            Self::Estimate => Ok(estimate(text, css_font)),
            #[cfg(test)]
            Self::Custom(f) => f(text, css_font).map_err(MeasureError::Other),
        }
    }
}

impl fmt::Debug for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fonts(m) => f.debug_tuple("Fonts").field(m).finish(),
            Self::NoFonts => f.write_str("NoFonts"),
            #[cfg(any(test, feature = "test-estimate"))]
            Self::Estimate => f.write_str("Estimate"),
            #[cfg(test)]
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

#[cfg(any(test, feature = "test-estimate"))]
/// Canvas's default font size.
const DEFAULT_FONT_PX: f64 = 10.0;

#[cfg(any(test, feature = "test-estimate"))]
/// The font size in px from a CSS `font` shorthand, or the canvas default.
fn font_size_px(css_font: &str) -> f64 {
    for token in css_font.split_whitespace() {
        // "12px/1.5" carries a line height after the slash.
        let size = token.split('/').next().unwrap_or(token);
        let (number, factor) = if let Some(n) = size.strip_suffix("px") {
            (n, 1.0)
        } else if let Some(n) = size.strip_suffix("pt") {
            (n, 4.0 / 3.0)
        } else if let Some(n) = size.strip_suffix("rem") {
            (n, 16.0)
        } else if let Some(n) = size.strip_suffix("em") {
            (n, 16.0)
        } else if let Some(n) = size.strip_suffix('%') {
            (n, 0.16)
        } else {
            continue;
        };
        if let Ok(v) = number.parse::<f64>() {
            if v.is_finite() && v >= 0.0 {
                return v * factor;
            }
        }
    }
    DEFAULT_FONT_PX
}

#[cfg(any(test, feature = "test-estimate"))]
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF
        | 0x20000..=0x3FFFD)
}

#[cfg(any(test, feature = "test-estimate"))]
fn estimate(text: &str, css_font: &str) -> TextMetrics {
    let em = font_size_px(css_font);
    let (wide, narrow) = text.chars().fold((0u32, 0u32), |(w, n), c| {
        if is_wide(c) { (w + 1, n) } else { (w, n + 1) }
    });
    // Fractions written as divisions so round sizes give round results.
    let width = 3.0 * f64::from(narrow) * em / 5.0 + f64::from(wide) * em;
    TextMetrics {
        width,
        actual_bounding_box_left: 0.0,
        actual_bounding_box_right: width,
        actual_bounding_box_ascent: 4.0 * em / 5.0,
        actual_bounding_box_descent: em / 5.0,
        font_bounding_box_ascent: 9.0 * em / 10.0,
        font_bounding_box_descent: em / 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_font_sizes() {
        assert_eq!(font_size_px("12px sans-serif"), 12.0);
        assert_eq!(font_size_px("bold 700 italic 14px/1.5 'Noto Sans'"), 14.0);
        assert_eq!(font_size_px("12pt serif"), 16.0);
        assert_eq!(font_size_px("2em x"), 32.0);
        assert_eq!(font_size_px("sans-serif"), DEFAULT_FONT_PX);
        assert_eq!(font_size_px(""), DEFAULT_FONT_PX);
        assert_eq!(font_size_px("NaNpx x"), DEFAULT_FONT_PX);
    }

    #[test]
    fn estimates_latin_and_cjk() {
        let m = Measure::Estimate;
        let latin = m.measure("abcde", "10px x").unwrap();
        assert!((latin.width - 30.0).abs() < 1e-9);
        let cjk = m.measure("日本語", "10px x").unwrap();
        assert!((cjk.width - 30.0).abs() < 1e-9);
        assert_eq!(cjk.actual_bounding_box_ascent, 8.0);
        assert_eq!(m.measure("", "10px x").unwrap().width, 0.0);
        assert_eq!(format!("{m:?}"), "Estimate");
    }

    #[test]
    fn no_fonts_is_its_own_error() {
        let empty = Measure::new(Some(TextMeasurer::new(surimono_gfx::FontSet::new())));
        assert_eq!(
            empty.measure("x", "10px sans-serif"),
            Err(MeasureError::NoFonts)
        );
        // Even for text the measurer would refuse as too long.
        let long = "x".repeat(surimono_gfx::MAX_TEXT_BYTES + 1);
        assert_eq!(
            empty.measure(&long, "10px sans-serif"),
            Err(MeasureError::NoFonts)
        );
        let none = Measure::new(None);
        assert_eq!(
            none.measure("x", "10px sans-serif"),
            Err(MeasureError::NoFonts)
        );
        assert_eq!(format!("{none:?}"), "NoFonts");
    }

    #[test]
    fn a_worker_measurer_measures_like_the_original() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts");
        let mut fonts = surimono_gfx::FontSet::new();
        fonts
            .add_font_bytes(std::fs::read(dir.join("NotoSans-Regular.ttf")).unwrap())
            .unwrap();
        let shared = Measure::new(Some(TextMeasurer::new(fonts)));
        let worker = shared.for_worker();
        for text in ["Revenue", "Q3 2026", ""] {
            assert_eq!(
                worker.measure(text, "12px 'Noto Sans'"),
                shared.measure(text, "12px 'Noto Sans'"),
                "{text:?}"
            );
        }
        assert!(matches!(Measure::NoFonts.for_worker(), Measure::NoFonts));
        assert!(matches!(Measure::Estimate.for_worker(), Measure::Estimate));
    }
}
