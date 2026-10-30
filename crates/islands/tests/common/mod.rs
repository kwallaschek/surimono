//! Helpers shared by the islands integration tests.
#![allow(dead_code)]

use std::path::Path;

use surimono_islands::{FontSet, TextMeasurer};

/// A measurer over the fixture fonts (Noto Sans and Noto Sans JP, regular
/// and bold), the fonts the visual fixtures print with.
pub fn fixture_measurer() -> TextMeasurer {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts");
    let mut fonts = FontSet::new();
    for file in [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSansJP-Regular.otf",
        "NotoSansJP-Bold.otf",
    ] {
        let bytes = std::fs::read(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        fonts.add_font_bytes(bytes).unwrap();
    }
    TextMeasurer::new(fonts)
}
