//! End to end on the checkpoint-2 fixtures (fixtures/islands, from P1-CF-03):
//! the real runner, the built-in renderer, the fixture fonts and the SVG
//! sanitiser together. The expectations here are structural (sanitised SVG,
//! document untouched around it, same bytes every time); how the charts look
//! is compared with Chrome's references by the fixture pipeline.
#![cfg(feature = "echarts")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use surimono::{Limits, Options, Renderer, TextMode};

const MARKER_TYPE: &str = "application/vnd.surimono+json";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> Vec<u8> {
    let path = repo().join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// These tests check structure, not speed: give every island far more than
/// the 2 s default, so a debug build under AddressSanitizer on a loaded CI
/// runner (several times slower than release) still finishes. Speed is the
/// benchmark's job.
fn structural_limits() -> Limits {
    Limits {
        time: Duration::from_secs(60),
        ..Limits::default()
    }
}

fn renderer() -> Renderer {
    let options = Options::new()
        .font(read("fixtures/fonts/NotoSans-Regular.ttf"))
        .font(read("fixtures/fonts/NotoSans-Bold.ttf"))
        .limits(structural_limits());
    Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
}

/// The input before and after its one island marker.
fn around_marker(input: &str) -> (&str, &str) {
    let start = input
        .find(&format!("<script type=\"{MARKER_TYPE}\""))
        .unwrap_or_else(|| panic!("no marker"));
    let end = input[start..]
        .find("</script>")
        .map(|i| start + i + "</script>".len())
        .unwrap_or_else(|| panic!("marker not closed"));
    (&input[..start], &input[end..])
}

#[test]
fn echarts_bar_simple_latin_becomes_sanitised_svg() {
    let input = String::from_utf8(read("fixtures/islands/echarts-bar-simple-latin/input.html"))
        .unwrap_or_else(|e| panic!("{e}"));
    let r = renderer();
    let out = r.render_islands(&input).unwrap_or_else(|e| panic!("{e}"));

    // Everything around the island is byte for byte the input.
    let (before, after) = around_marker(&input);
    assert!(out.starts_with(before) && out.ends_with(after));
    let island = &out[before.len()..out.len() - after.len()];
    assert!(!out.contains(MARKER_TYPE));

    // One wrapper holding one SVG.
    assert!(
        island.starts_with("<div class=\"surimono-island\" data-renderer=\"echarts\"><svg"),
        "{}",
        &island[..island.len().min(200)]
    );
    assert!(island.ends_with("</svg></div>"));
    assert_eq!(island.matches("<svg").count(), 1);

    // Sanitised: no scripts, handlers or external references; ids carry the
    // island's prefix; text is set in the registered fonts.
    let lower = island.to_ascii_lowercase();
    for banned in [
        "<script",
        "javascript:",
        "<foreignobject",
        "http://",
        "https://",
    ] {
        let allowed_ns =
            lower.matches(banned).count() == lower.matches(&format!("{banned}www.w3.org")).count();
        assert!(
            !lower.contains(banned) || (banned.starts_with("http") && allowed_ns),
            "{banned} in the island"
        );
    }
    for (i, _) in lower.match_indices(" on") {
        let name: String = lower[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        assert!(
            !lower[i + 1 + name.len()..].starts_with('='),
            "event handler {name} in the island"
        );
    }
    for (i, _) in island.match_indices(" id=\"") {
        assert!(island[i + 5..].starts_with("sm0-"), "unprefixed id");
    }
    // Text is outlined in the fixture font by default (TextMode::Outlines):
    // no <text> is left, and the words stay as labels for accessibility.
    assert!(!lower.contains("<text"), "text left as <text>");
    assert!(lower.contains("<path"));
    assert!(
        island.contains("Invoiced revenue (EUR)"),
        "chart title missing"
    );

    // Same input, same output: again, and from a fresh renderer.
    assert_eq!(r.render_islands(&input).ok().as_deref(), Some(out.as_str()));
    assert_eq!(
        renderer().render_islands(&input).ok().as_deref(),
        Some(out.as_str())
    );
}

/// Two charts on one page: each island's ids get its own prefix, so the second
/// chart's clip paths and gradients can't resolve to the first chart's. (The
/// line chart has clip paths; ECharts names them zr0-… in every chart.)
#[test]
fn two_echarts_islands_share_no_ids() {
    let input = String::from_utf8(read("fixtures/islands/echarts-line-dense-latin/input.html"))
        .unwrap_or_else(|e| panic!("{e}"));
    let (before, after) = around_marker(&input);
    let marker = &input[before.len()..input.len() - after.len()];
    let doubled = format!("{before}{marker}\n{marker}{after}");
    let out = renderer()
        .render_islands(&doubled)
        .unwrap_or_else(|e| panic!("{e}"));
    let ids: Vec<&str> = out
        .match_indices(" id=\"")
        .filter_map(|(i, m)| out[i + m.len()..].split('"').next())
        .collect();
    assert!(ids.iter().any(|id| id.starts_with("sm0-")));
    assert!(ids.iter().any(|id| id.starts_with("sm1-")));
    let mut seen = std::collections::HashSet::new();
    for id in &ids {
        assert!(seen.insert(*id), "id {id:?} appears twice");
    }
}

/// With TextMode::Keep, text stays text, set in the registered family.
#[test]
fn keep_mode_keeps_text_in_the_registered_font() {
    let input = String::from_utf8(read("fixtures/islands/echarts-bar-simple-latin/input.html"))
        .unwrap_or_else(|e| panic!("{e}"));
    let options = Options::new()
        .font(read("fixtures/fonts/NotoSans-Regular.ttf"))
        .font(read("fixtures/fonts/NotoSans-Bold.ttf"))
        .limits(structural_limits())
        .text_mode(TextMode::Keep);
    let out = Renderer::new(options)
        .and_then(|r| r.render_islands(&input))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(out.contains("<text"));
    assert!(out.contains("Noto Sans"));
}

/// Every island fixture (the ten for checkpoint 2, ECharts, Vega and Vega-Lite,
/// simple and dense, Latin and Japanese, plus any added since) renders with
/// all four fixture fonts: every marker becomes one sanitised, outlined SVG,
/// and a second render gives the same bytes.
#[test]
#[cfg(all(feature = "echarts", feature = "vega"))]
fn every_checkpoint_fixture_renders() {
    let options = [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSansJP-Regular.otf",
        "NotoSansJP-Bold.otf",
    ]
    .iter()
    .fold(Options::new().limits(structural_limits()), |o, f| {
        o.font(read(&format!("fixtures/fonts/{f}")))
    });
    let r = Renderer::new(options).unwrap_or_else(|e| panic!("{e}"));
    let dir = repo().join("fixtures/islands");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("input.html").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 10, "{names:?}");
    for name in names {
        let input = String::from_utf8(read(&format!("fixtures/islands/{name}/input.html")))
            .unwrap_or_else(|e| panic!("{e}"));
        let markers = input.matches(MARKER_TYPE).count();
        let out = r
            .render_islands(&input)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!out.contains(MARKER_TYPE), "{name}");
        assert_eq!(
            out.matches("<div class=\"surimono-island\"").count(),
            markers,
            "{name}"
        );
        let lower = out.to_ascii_lowercase();
        assert!(!lower.contains("<text"), "{name}: text not outlined");
        assert_eq!(
            r.render_islands(&input).ok().as_deref(),
            Some(out.as_str()),
            "{name}"
        );
    }
}
