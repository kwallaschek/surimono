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

/// An island marker found in a document.
pub struct Island {
    /// Where it came from (a fixture name or a file name).
    pub source: String,
    pub renderer: String,
    pub width: u32,
    pub height: u32,
    pub spec: String,
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let len = tag[start..].find('"')?;
    Some(tag[start..start + len].to_owned())
}

/// The island markers in `html`, in document order: `<script
/// type="application/vnd.surimono+json" data-renderer=… data-width=…
/// data-height=…>`. A simple scanner for surimono's own fixtures, not an
/// HTML parser.
pub fn islands_in(source: &str, html: &str) -> Vec<Island> {
    const MARKER: &str = "type=\"application/vnd.surimono+json\"";
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let after = &rest[open..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let tag = &after[..tag_end];
        let body_start = tag_end + 1;
        let Some(close) = after[body_start..].find("</script>") else {
            break;
        };
        if tag.contains(MARKER) {
            if let (Some(renderer), Some(w), Some(h)) = (
                attribute(tag, "data-renderer"),
                attribute(tag, "data-width"),
                attribute(tag, "data-height"),
            ) {
                out.push(Island {
                    source: source.to_owned(),
                    renderer,
                    width: w.parse().unwrap_or(0),
                    height: h.parse().unwrap_or(0),
                    spec: after[body_start..body_start + close].trim().to_owned(),
                });
            }
        }
        rest = &after[body_start + close..];
    }
    out
}

/// Every island in the visual fixtures: each `*/input.html` under
/// `SURIMONO_FIXTURES_DIR` (default: the repository's `fixtures/islands`),
/// fixtures sorted by name.
pub fn fixture_islands() -> Vec<Island> {
    let dir = std::env::var_os("SURIMONO_FIXTURES_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/islands"),
        std::path::PathBuf::from,
    );
    let mut fixtures: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("input.html").exists())
        .collect();
    fixtures.sort();
    let mut islands = Vec::new();
    for f in &fixtures {
        let html = std::fs::read_to_string(f.join("input.html")).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        islands.extend(islands_in(&name, &html));
    }
    assert!(!islands.is_empty(), "no islands under {}", dir.display());
    islands
}

/// The test time-scale factor: `SURIMONO_TEST_TIME_SCALE`, an integer of at
/// least 1, default 1. Slow builds (AddressSanitizer in
/// `scripts/ci/asan.sh`) set it so wall-clock limits and bounds in the
/// tests stretch with the build. It never changes the library's defaults;
/// only tests read it. A value that is not an integer of at least 1 fails
/// the test, so a typo can't silently run unscaled. (The same helper as in
/// `crates/core/tests/common`.)
pub fn time_scale() -> u32 {
    match std::env::var("SURIMONO_TEST_TIME_SCALE") {
        Err(std::env::VarError::NotPresent) => 1,
        Ok(s) if s.is_empty() => 1,
        Ok(s) => match s.parse::<u32>() {
            Ok(n) if n >= 1 => n,
            _ => panic!("SURIMONO_TEST_TIME_SCALE must be an integer of at least 1, not {s:?}"),
        },
        Err(e) => panic!("SURIMONO_TEST_TIME_SCALE: {e}"),
    }
}

/// `d` times [`time_scale`].
pub fn scaled(d: std::time::Duration) -> std::time::Duration {
    d * time_scale()
}
