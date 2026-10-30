//! Text metrics from the renderer's fonts, the Canvas2D recorder, and SVG output.
//!
//! Part of surimono, a library that runs a document's
//! JavaScript without a browser. See `docs/BRIEF.md` for the design.

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
