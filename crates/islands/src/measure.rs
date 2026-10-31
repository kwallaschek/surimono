//! Text measurement for renderers, behind `ctx.measureText(text, cssFont)`.
//!
//! The real measurer comes from surimono-gfx (the same fonts the PDF engine
//! prints with). This module defines the shape islands needs and a crude,
//! deterministic stand-in so the runner works without fonts.

use std::fmt;
use std::sync::Arc;

/// Canvas-style text metrics in CSS pixels, as returned to JavaScript by
/// `ctx.measureText`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    /// Advance width.
    pub width: f64,
    /// Distance from the alignment point to the left of the ink.
    pub actual_bounding_box_left: f64,
    /// Distance from the alignment point to the right of the ink.
    pub actual_bounding_box_right: f64,
    /// Distance from the baseline to the top of the ink.
    pub actual_bounding_box_ascent: f64,
    /// Distance from the baseline to the bottom of the ink.
    pub actual_bounding_box_descent: f64,
    /// The font's ascent.
    pub font_bounding_box_ascent: f64,
    /// The font's descent.
    pub font_bounding_box_descent: f64,
}

type MeasureFn = dyn Fn(&str, &str) -> Result<TextMetrics, String> + Send + Sync;

/// Measures text for renderers: `measure(text, css_font)`.
///
/// Cheap to clone. [`TextMeasurer::default`] is [`TextMeasurer::approximate`].
#[derive(Clone)]
pub struct TextMeasurer {
    f: Arc<MeasureFn>,
    name: &'static str,
}

impl TextMeasurer {
    /// Wraps a measuring function. It gets the text and a CSS `font`
    /// shorthand such as `"bold 12px Noto Sans"`; an `Err` becomes a
    /// `TypeError` in JavaScript.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&str, &str) -> Result<TextMetrics, String> + Send + Sync + 'static,
    {
        Self {
            f: Arc::new(f),
            name: "custom",
        }
    }

    /// A deterministic stand-in that needs no fonts: 0.6 em per character
    /// (1 em for CJK and other wide characters), ascent 0.8 em, descent
    /// 0.2 em. Good enough for tests; real documents use the gfx measurer.
    pub fn approximate() -> Self {
        Self {
            f: Arc::new(|text, font| Ok(approximate(text, font))),
            name: "approximate",
        }
    }

    /// Measures `text` in `css_font`.
    pub fn measure(&self, text: &str, css_font: &str) -> Result<TextMetrics, String> {
        (self.f)(text, css_font)
    }
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::approximate()
    }
}

impl fmt::Debug for TextMeasurer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TextMeasurer").field(&self.name).finish()
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

fn approximate(text: &str, css_font: &str) -> TextMetrics {
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
    fn approximates_latin_and_cjk() {
        let m = TextMeasurer::approximate();
        let latin = m.measure("abcde", "10px x").unwrap();
        assert!((latin.width - 30.0).abs() < 1e-9);
        let cjk = m.measure("日本語", "10px x").unwrap();
        assert!((cjk.width - 30.0).abs() < 1e-9);
        assert_eq!(cjk.actual_bounding_box_ascent, 8.0);
        assert_eq!(m.measure("", "10px x").unwrap().width, 0.0);
    }

    #[test]
    fn custom_measurers_are_called() {
        let m = TextMeasurer::new(|text, font| {
            if font.is_empty() {
                return Err("no font".into());
            }
            Ok(TextMetrics {
                width: text.len() as f64,
                ..TextMetrics::default()
            })
        });
        assert_eq!(m.measure("abc", "x").unwrap().width, 3.0);
        assert_eq!(m.measure("abc", "").unwrap_err(), "no font");
        assert_eq!(format!("{m:?}"), "TextMeasurer(\"custom\")");
    }
}
