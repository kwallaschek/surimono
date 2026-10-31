//! The `.idl` files vendored from w3c/webref (`ed/idl/`) at a pinned commit.
//!
//! Builds and tests never touch the network. To move the pin, run
//! `crates/webidl/fetch-webref.sh <commit>` by hand, update [`WEBREF_COMMIT`]
//! and `idl/SOURCES.md`, regenerate the stubs and review the diff.
//! Licences: `idl/SOURCES.md`.

/// The w3c/webref commit the files were copied from.
pub const WEBREF_COMMIT: &str = "09f7739aa691c3b9784f1e45069b8fa375e61423";

/// `(file name, contents)` for every vendored file, sorted by file name.
pub const FILES: &[(&str, &str)] = &[
    ("SVG.idl", include_str!("../idl/SVG.idl")),
    ("console.idl", include_str!("../idl/console.idl")),
    (
        "css-font-loading.idl",
        include_str!("../idl/css-font-loading.idl"),
    ),
    ("cssom-view.idl", include_str!("../idl/cssom-view.idl")),
    ("cssom.idl", include_str!("../idl/cssom.idl")),
    ("dom.idl", include_str!("../idl/dom.idl")),
    ("encoding.idl", include_str!("../idl/encoding.idl")),
    ("geometry.idl", include_str!("../idl/geometry.idl")),
    ("hr-time.idl", include_str!("../idl/hr-time.idl")),
    ("html.idl", include_str!("../idl/html.idl")),
    (
        "resize-observer.idl",
        include_str!("../idl/resize-observer.idl"),
    ),
    ("uievents.idl", include_str!("../idl/uievents.idl")),
    ("url.idl", include_str!("../idl/url.idl")),
    ("webidl.idl", include_str!("../idl/webidl.idl")),
];

/// Parses every vendored file into one [`crate::IdlSet`].
pub fn idl_set() -> Result<crate::IdlSet, crate::SetError> {
    crate::IdlSet::parse_sources(FILES.iter().copied())
}
