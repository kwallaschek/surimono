//! Text metrics from the renderer's fonts, the Canvas2D recorder, and SVG output.
//!
//! Part of surimono, a library that runs a document's JavaScript without a
//! browser. This crate answers canvas `measureText` with the caller's fonts and
//! the shaper fulgur 0.40 lays out HTML text with (Parley 0.6), so chart labels
//! measured in the sandbox fit when printed.
//!
//! fulgur 0.40 prints HTML text with these fonts, but inline SVG text goes
//! through usvg with system fonts; see `HANDOFF.md` for how island text still
//! prints exactly as measured.
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

#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

mod css_font;
mod error;
mod fonts;
mod measure;
mod svg;

pub use css_font::GenericFamily;
pub use error::GfxError;
pub use fonts::FontSet;
pub use measure::{MAX_TEXT_BYTES, TextMeasurer, TextMetrics};
pub use svg::{MAX_SVG_DEPTH, MAX_SVG_NODES, SvgOptions, TextMode, sanitize_svg};

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
