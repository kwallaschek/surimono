//! Print media in the printer-safe output (Q-PKG-03), held against what
//! Chromium 153 prints: `tests/data/print-media/`, measured by `measure.py`
//! there. No browser or printer runs here; the test checks that the committed
//! measurements are of this code's output, and what they say.
//!
//! The expectation is always Chromium's print of the original page: the
//! labels (the words of its `<p>` elements, each shown by one media query or
//! sheet) Chromium prints. Two things are required of the output:
//!
//! 1. Chromium prints it exactly as the original: the same labels, and the
//!    same pixels at 96 dpi on every page ("neutral in Chrome").
//! 2. Each printer (fulgur 0.40.0, the pinned printer, and Raikiri) prints it
//!    no further from Chromium than main's output: no more labels that
//!    Chromium doesn't print, or missing that it does. On the pages where the
//!    media type is all that decides, fulgur prints Chromium's labels exactly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use surimono::{Options, Renderer, ResourceLoader};

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/print-media")
}

/// Every page of the matrix, as `measure.py` finds them.
const PAGES: &[&str] = &[
    "features",
    "import-media",
    "invalid",
    "layers",
    "link-media",
    "nesting",
    "report",
    "style-media",
    "style-media-selector",
    "types",
];

/// Pages where fulgur prints Chromium's labels exactly from the output. On
/// the others something besides the media type stays: `features` asks the
/// printer about the page (fulgur's width is its page area, 680 px, where
/// Chromium's is the paper, 816 px; it answers `color`, `hover`,
/// `orientation` and `resolution` otherwise), `report` prints a
/// `content: attr()` differently, and `import-media` imports from `data:`
/// URLs, which neither printer loads, and an `@import` is CSS that wasn't
/// read, so nothing is resolved there. `nesting` is back since Q-PKG-08: a
/// rule nested in a style rule is matched by its last compound, and a
/// keyframe with no animation anywhere matches nothing, so its one
/// `display: block` in `@keyframes` no longer may show a `<style>`.
const EXACT_ON_FULGUR: &[&str] = &[
    "invalid",
    "layers",
    "link-media",
    "nesting",
    "style-media",
    "style-media-selector",
    "types",
];

fn renderer() -> Renderer {
    let font =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts/NotoSans-Regular.ttf");
    let bytes = std::fs::read(&font).unwrap_or_else(|e| panic!("{}: {e}", font.display()));
    let names = ["link-not-print.css", "link-print.css", "link-screen.css"];
    let loader = ResourceLoader::new(names, |url: &str| {
        std::fs::read(data().join(url)).map_err(|e| e.to_string())
    });
    Renderer::new(Options::new().font(bytes).resource_loader(loader))
        .unwrap_or_else(|e| panic!("{e}"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn labels(v: &Value) -> BTreeSet<String> {
    let list = v.as_array().unwrap_or_else(|| panic!("not a list: {v}"));
    list.iter()
        .map(|l| l.as_str().unwrap_or_default().to_owned())
        .collect()
}

/// How many labels `printed` has that `expected` hasn't, or lacks.
fn off(expected: &BTreeSet<String>, printed: &BTreeSet<String>) -> usize {
    expected.symmetric_difference(printed).count()
}

#[test]
fn the_measurements_are_of_this_output() {
    let r = renderer();
    for page in PAGES {
        let html = read(&data().join(format!("{page}.html")));
        let out = r
            .render_page(&html)
            .unwrap_or_else(|e| panic!("{page}: {e}"));
        let measured = read(&data().join(format!("{page}.output.html")));
        assert_eq!(out.html, measured, "{page}: run measure.py again");
    }
}

#[test]
fn chromium_prints_the_output_as_the_original() {
    for page in PAGES {
        let m: Value = serde_json::from_str(&read(&data().join(format!("{page}.measured.json"))))
            .unwrap_or_else(|e| panic!("{page}: {e}"));
        assert!(
            m["chromium"]
                .as_str()
                .is_some_and(|v| v.starts_with("153.")),
            "{page}"
        );
        let original = labels(&m["chrome"]["original"]);
        assert!(!original.is_empty(), "{page}");
        assert_eq!(labels(&m["chrome"]["output"]), original, "{page}");
        assert_eq!(m["pixels_identical"]["output"], Value::Bool(true), "{page}");
        assert_eq!(m["pixels_identical"]["main"], Value::Bool(true), "{page}");
    }
}

#[test]
fn the_printers_come_no_further_from_chromium() {
    let (mut before, mut after) = (0, 0);
    for page in PAGES {
        let m: Value = serde_json::from_str(&read(&data().join(format!("{page}.measured.json"))))
            .unwrap_or_else(|e| panic!("{page}: {e}"));
        let chrome = labels(&m["chrome"]["original"]);
        for printer in ["fulgur", "raikiri"] {
            let printed = &m[printer]["labels"];
            let (was, is) = (
                off(&chrome, &labels(&printed["main"])),
                off(&chrome, &labels(&printed["output"])),
            );
            assert!(
                is <= was,
                "{page} with {printer}: {was} labels off before, {is} now"
            );
            if printer == "fulgur" {
                before += was;
                after += is;
            }
        }
        if EXACT_ON_FULGUR.contains(page) {
            assert_eq!(labels(&m["fulgur"]["labels"]["output"]), chrome, "{page}");
        }
    }
    assert!(
        after < before,
        "fulgur: {before} labels off before, {after} now"
    );
}
