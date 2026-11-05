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

/// The length of the number at the front of `s`: digits, and a fraction if
/// there is one. 0 if `s` doesn't start with a digit.
fn number_len(s: &str) -> usize {
    let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let whole = digits(s);
    match s[whole..].strip_prefix('.') {
        Some(rest) if whole > 0 && digits(rest) > 0 => whole + 1 + digits(rest),
        _ => whole,
    }
}

/// `ours` with each fraction within 1e-9 (relative) of the one at the same
/// place in `expected` replaced by that one; everything else stays, so a real
/// difference still shows up in the comparison.
fn adopt_close_floats(ours: &str, expected: &str) -> String {
    let (mut a, mut b) = (ours, expected);
    let mut out = String::with_capacity(ours.len());
    loop {
        let mut same = a
            .bytes()
            .zip(b.bytes())
            .take_while(|(x, y)| x == y && !x.is_ascii_digit())
            .count();
        // Two different characters can share their first bytes.
        while !a.is_char_boundary(same) {
            same -= 1;
        }
        out.push_str(&a[..same]);
        (a, b) = (&a[same..], &b[same..]);
        let (m, n) = (number_len(a), number_len(b));
        if m == 0 || n == 0 {
            out.push_str(a);
            return out;
        }
        let (x, y) = (&a[..m], &b[..n]);
        let close = match (x.parse::<f64>(), y.parse::<f64>()) {
            (Ok(p), Ok(q)) => (p - q).abs() <= 1e-9 * p.abs().max(q.abs()),
            _ => false,
        };
        let fractions = x.contains('.') || y.contains('.');
        out.push_str(if close && fractions { y } else { x });
        (a, b) = (&a[m..], &b[n..]);
    }
}

/// Math.sin and the like come from the platform's libm in QuickJS and from a
/// fixed port in V8, so a page's computed coordinates can differ from Chrome's
/// in the last digits on another libm (Apple's: `d3`'s centroids, one ulp). On
/// x86_64 Linux, where the references hold exactly, nothing is forgiven; a
/// fixed libm in the engine would make this unneeded (docs/COMPAT.md).
const EXACT_FLOATS: bool = cfg!(all(target_os = "linux", target_arch = "x86_64"));

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
    let ours = if EXACT_FLOATS {
        ours
    } else {
        adopt_close_floats(&ours, &expected)
    };
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
fn only_close_fractions_are_adopted() {
    let adopt = |a, b| adopt_close_floats(a, b);
    assert_eq!(
        adopt("(-52.15729913154639,1)", "(-52.157299131546395,1)"),
        "(-52.157299131546395,1)"
    );
    // A real difference, in a fraction or a whole number, stays.
    assert_eq!(adopt("x=1.5 y", "x=1.6 y"), "x=1.5 y");
    assert_eq!(adopt("h1", "h2"), "h1");
    assert_eq!(adopt("1000000000000", "1000000000001"), "1000000000000");
    // So does anything after the first place the text parts apart.
    assert_eq!(adopt("a 1.5 b 2.5", "a 1.5 c 2.50000000001"), "a 1.5 b 2.5");
    assert_eq!(
        adopt("0.1 1", "0.1 1.0000000000000002"),
        "0.1 1.0000000000000002"
    );
    assert_eq!(adopt("", "1.5"), "");
    // あ and い share two bytes.
    assert_eq!(adopt("あ1.5", "い1.5"), "あ1.5");
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

#[test]
fn noscript_message() {
    check("noscript-message");
}
