//! Text metrics from the renderer's fonts, the Canvas2D recorder, and SVG output.
//!
//! Part of surimono, a library that runs a document's JavaScript without a
//! browser. This crate answers canvas `measureText` with the same fonts and the
//! same shaper (Parley 0.6, as in fulgur 0.40) that print the document, so
//! chart labels measured in the sandbox fit when printed.
//!
//! ```no_run
//! use surimono_gfx::{FontSet, TextMeasurer};
//!
//! let mut fonts = FontSet::new();
//! fonts.add_font_bytes(std::fs::read("NotoSans-Regular.ttf")?)?;
//! let measurer = TextMeasurer::new(fonts);
//! let m = measurer.measure("Revenue", "bold 12px sans-serif")?;
//! println!("{} px wide", m.width);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod css_font;
mod error;
mod fonts;
mod measure;
mod svg;

pub use css_font::GenericFamily;
pub use error::GfxError;
pub use fonts::FontSet;
pub use measure::{MAX_TEXT_BYTES, TextMeasurer, TextMetrics};
pub use svg::{MAX_SVG_DEPTH, MAX_SVG_NODES, sanitize_svg};

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
