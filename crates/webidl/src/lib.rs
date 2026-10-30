//! WebIDL parsing and binding generation from w3c/webref.
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
