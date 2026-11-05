//! The phase-2 real-world pages (fixtures/real-world, E2-10) run as pages:
//! their own `<script>` tags, resolved to the vendored libraries. Expected
//! DOM is Chrome's, `reference/dom.html`. Runs once those fixtures are on the
//! branch, or with SURIMONO_REAL_WORLD pointing at a checkout of them.

mod common;

use std::path::{Path, PathBuf};

use common::{limits, scaled};
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomRuntime};

fn root() -> PathBuf {
    match std::env::var_os("SURIMONO_REAL_WORLD") {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real-world"),
    }
}

/// Runs the page; returns our serialized `<html>`, Chrome's, and the report.
fn run(name: &str) -> Option<(String, String, surimono_dom::PageReport)> {
    let dir = root().join(name);
    let input = std::fs::read_to_string(dir.join("input.html")).ok()?;
    let expected = std::fs::read_to_string(dir.join("reference/dom.html")).ok()?;
    let mut rt = DomRuntime::new(
        limits()
            .with_time(scaled(std::time::Duration::from_secs(60)))
            .with_memory_bytes(512 << 20),
    )
    .unwrap();
    rt.load(Document::parse(&input).unwrap());
    let report = rt
        .run_page(&SettleOptions::default(), |src| {
            std::fs::read_to_string(dir.join(src)).ok()
        })
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    Some((
        rt.with_document(Document::to_html).unwrap(),
        expected,
        report,
    ))
}

/// Whitespace between tags doesn't matter here; attribute order and the
/// rest do.
fn squeeze(html: &str) -> String {
    html.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// Moves each tag's `style` attribute to the end of the tag, on both sides.
/// Chrome adds `style` lazily, when something reads the attributes, so a
/// `style` set through `element.style` lands after attributes set later. CSSOM
/// §6.6 updates the attribute at once, which is what we do; the spec wins for
/// DOM behaviour, and order doesn't change how the page looks.
fn style_last(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(" style=\"") {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let body = &tail[" style=\"".len()..];
        let (Some(end), Some(close)) = (body.find('"'), tail.find('>')) else {
            out.push_str(tail);
            return out;
        };
        let attr = &tail[..end + " style=\"\"".len()];
        out.push_str(&tail[attr.len()..close]);
        out.push_str(attr);
        rest = &tail[close..];
    }
    out.push_str(rest);
    out
}

fn check(name: &str) {
    let Some((ours, expected, report)) = run(name) else {
        eprintln!("{name}: no fixture here, skipped");
        return;
    };
    assert_eq!(
        report.uncaught, 0,
        "{name}: {:?} {:?}",
        report.errors, report.settle.errors
    );
    assert!(report.missing_scripts.is_empty(), "{name}");
    let (ours, expected) = (style_last(&squeeze(&ours)), style_last(&squeeze(&expected)));
    // `to_html` is the `<html>` element's outer HTML.
    let expected = expected
        .strip_prefix("<!doctype html>")
        .unwrap_or(&expected)
        .to_owned();
    if let Ok(dir) = std::env::var("SURIMONO_REAL_WORLD_OUT") {
        std::fs::write(format!("{dir}/{name}.ours.html"), &ours).unwrap();
        std::fs::write(format!("{dir}/{name}.chrome.html"), &expected).unwrap();
    }
    let at = ours
        .bytes()
        .zip(expected.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(ours.len().min(expected.len()));
    let around = |h: &str| {
        let bytes = h.as_bytes();
        let range = at.saturating_sub(60)..(at + 120).min(bytes.len());
        String::from_utf8_lossy(&bytes[range]).into_owned()
    };
    assert!(
        ours == expected,
        "{name}: differs at byte {at}:\n ours: {}\n chrome: {}",
        around(&ours),
        around(&expected),
    );
}

#[test]
fn marked() {
    check("marked-report");
}

#[test]
fn highlight_js() {
    check("highlightjs-code");
}

#[test]
fn katex() {
    check("katex-autorender");
}

#[test]
fn d3() {
    check("d3-charts");
}

#[test]
fn alpine() {
    check("alpine-invoice");
}
