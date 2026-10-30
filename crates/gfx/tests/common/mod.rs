//! Shared test helpers: the pinned test fonts from `fixtures/fonts/`.

#![allow(dead_code)]

use std::path::PathBuf;

use surimono_gfx::FontSet;

/// The shared test fonts, in the order they are registered.
pub const FONT_FILES: [&str; 4] = [
    "NotoSans-Regular.ttf",
    "NotoSans-Bold.ttf",
    "NotoSansJP-Regular.otf",
    "NotoSansJP-Bold.otf",
];

/// `fixtures/fonts/`, or `$SURIMONO_TEST_FONTS` when set.
pub fn fonts_dir() -> PathBuf {
    match std::env::var_os("SURIMONO_TEST_FONTS") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts"),
    }
}

/// Reads one test font. Panics with a clear message if it is missing.
pub fn font_bytes(file: &str) -> Vec<u8> {
    let path = fonts_dir().join(file);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "test font {} is missing ({e}); it comes from fixtures/fonts \
             (see fixtures/fonts/fonts.toml)",
            path.display()
        )
    })
}

/// A font set with all four test fonts, Latin first.
pub fn test_fonts() -> FontSet {
    let mut fonts = FontSet::new();
    for file in FONT_FILES {
        let faces = fonts.add_font_bytes(font_bytes(file)).expect(file);
        assert_eq!(faces, 1, "{file}");
    }
    fonts
}
