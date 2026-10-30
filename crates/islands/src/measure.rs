//! Text measurement for renderers, behind `ctx.measureText(text, cssFont)`.
//!
//! Real documents measure with [`surimono_gfx::TextMeasurer`]: the caller's
//! fonts, shaped as fulgur prints them. Without one, a runner measures with a
//! deterministic, font-free estimate, which is only good for tests and
//! tooling.

use std::fmt;
#[cfg(test)]
use std::sync::Arc;

use surimono_gfx::{TextMeasurer, TextMetrics};

/// How a runner's workers answer `ctx.measureText`.
#[derive(Clone)]
pub(crate) enum Measure {
    /// The caller's fonts.
    Fonts(TextMeasurer),
    /// The font-free estimate: 0.6 em per character, 1 em for wide (CJK)
    /// characters, ascent 0.8 em, descent 0.2 em.
    Estimate,
    /// Any function, for the runner's own tests (errors, panics).
    #[cfg(test)]
    Custom(Arc<CustomMeasure>),
}

/// A measuring function for tests.
#[cfg(test)]
pub(crate) type CustomMeasure = dyn Fn(&str, &str) -> Result<TextMetrics, String> + Send + Sync;

impl Measure {
    pub(crate) fn new(measurer: Option<TextMeasurer>) -> Self {
        measurer.map_or(Self::Estimate, Self::Fonts)
    }

    /// Measures `text` in the CSS `font` shorthand `css_font`; an error
    /// becomes a `TypeError` in JavaScript.
    pub(crate) fn measure(&self, text: &str, css_font: &str) -> Result<TextMetrics, String> {
        match self {
            Self::Fonts(m) => m.measure(text, css_font).map_err(|e| e.to_string()),
            Self::Estimate => Ok(estimate(text, css_font)),
            #[cfg(test)]
            Self::Custom(f) => f(text, css_font),
        }
    }
}

impl fmt::Debug for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fonts(m) => f.debug_tuple("Fonts").field(m).finish(),
            Self::Estimate => f.write_str("Estimate"),
            #[cfg(test)]
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

/// Canvas's default font size.
const DEFAULT_FONT_PX: f64 = 10.0;

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
        let m = Measure::new(None);
        let latin = m.measure("abcde", "10px x").unwrap();
        assert!((latin.width - 30.0).abs() < 1e-9);
        let cjk = m.measure("日本語", "10px x").unwrap();
        assert!((cjk.width - 30.0).abs() < 1e-9);
        assert_eq!(cjk.actual_bounding_box_ascent, 8.0);
        assert_eq!(m.measure("", "10px x").unwrap().width, 0.0);
        assert_eq!(format!("{m:?}"), "Estimate");
    }

    #[test]
    fn font_errors_are_messages() {
        // A measurer with no fonts can't measure anything.
        let m = Measure::new(Some(TextMeasurer::new(surimono_gfx::FontSet::new())));
        let err = m.measure("x", "10px sans-serif").unwrap_err();
        assert!(!err.is_empty());
    }
}
