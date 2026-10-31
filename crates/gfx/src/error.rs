//! Errors returned by surimono-gfx.

/// An error from the font registry or text measurement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GfxError {
    /// The bytes are not a font this crate can read (TTF, OTF or TTC).
    #[error("not a supported font file: {reason}")]
    InvalidFont {
        /// Why the bytes were rejected.
        reason: String,
    },

    /// The font container is recognised but not supported (for example WOFF2).
    #[error("unsupported font format {format}; decode it to TTF or OTF first")]
    UnsupportedFontFormat {
        /// The detected container format.
        format: &'static str,
    },

    /// A family name was used that no registered font provides.
    #[error("no registered font has the family name {family:?}")]
    UnknownFamily {
        /// The family name as given.
        family: String,
    },

    /// Text was measured before any font was registered.
    #[error("no fonts are registered; add at least one font before measuring text")]
    NoFonts,

    /// SVG that can't be parsed or rewritten safely.
    #[error("invalid SVG: {reason}")]
    InvalidSvg {
        /// What was wrong.
        reason: String,
    },

    /// The text to measure is longer than the measurer accepts.
    #[error("text is {len} bytes long; the limit is {max} bytes")]
    TextTooLong {
        /// Length of the text in bytes.
        len: usize,
        /// The limit in bytes.
        max: usize,
    },
}
