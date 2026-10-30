//! measureText against Chrome's canvas, with the same font files.
//!
//! The expected numbers in `tests/data/chrome_measure_text.json` come from
//! `tests/chrome/measure_text.mjs` (Chromium via Playwright). Never edit them
//! to match our output; a real difference goes in docs/COMPAT.md.

mod common;

use serde_json::Value;
use surimono_gfx::{FontSet, GenericFamily, GfxError, TextMeasurer, TextMetrics};

/// Advance widths: Chrome and Parley shape with the same rules (harfrust is a
/// port of HarfBuzz) from the same font, so they agree up to float rounding.
/// Chrome carries positions through 16.16 fixed point, about 1e-4 px a glyph;
/// 0.01 px covers 100 glyphs. Observed: 0.0007 px.
const WIDTH_TOLERANCE: f64 = 0.01;
/// Font ascent and descent: both sides round the same font metric to whole px.
const FONT_BOX_TOLERANCE: f64 = 0.0;
/// Ink bounds: both sides snap each glyph's box outwards to whole px and add
/// the glyph's fractional position, so they differ only as much as positions
/// do. A glyph edge that FreeType's hinting moves across a pixel boundary would
/// differ by 1 px; none of these cases does (observed: 0.0006 px).
const INK_TOLERANCE: f64 = 0.01;

/// A metric's name in the JSON, how to read it from ours, and its tolerance.
type Field = (&'static str, fn(&TextMetrics) -> f64, f64);

fn chrome_reference() -> Value {
    let raw = include_str!("data/chrome_measure_text.json");
    serde_json::from_str(raw).expect("valid JSON")
}

fn get(case: &Value, key: &str) -> f64 {
    case[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} in {case}"))
}

#[test]
fn reference_fonts_are_the_ones_chrome_used() {
    let reference = chrome_reference();
    for font in reference["fonts"].as_array().expect("fonts") {
        let file = font["file"].as_str().expect("file");
        let bytes = common::font_bytes(file);
        assert_eq!(
            bytes.len() as u64,
            font["bytes"].as_u64().expect("bytes"),
            "{file} differs from the font Chrome measured with"
        );
    }
}

#[test]
fn matches_chrome_measure_text() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let reference = chrome_reference();
    let cases = reference["cases"].as_array().expect("cases");
    assert!(cases.len() >= 20);

    let fields: [Field; 7] = [
        ("width", |m| m.width, WIDTH_TOLERANCE),
        (
            "font_bounding_box_ascent",
            |m| m.font_bounding_box_ascent,
            FONT_BOX_TOLERANCE,
        ),
        (
            "font_bounding_box_descent",
            |m| m.font_bounding_box_descent,
            FONT_BOX_TOLERANCE,
        ),
        (
            "actual_bounding_box_left",
            |m| m.actual_bounding_box_left,
            INK_TOLERANCE,
        ),
        (
            "actual_bounding_box_right",
            |m| m.actual_bounding_box_right,
            INK_TOLERANCE,
        ),
        (
            "actual_bounding_box_ascent",
            |m| m.actual_bounding_box_ascent,
            INK_TOLERANCE,
        ),
        (
            "actual_bounding_box_descent",
            |m| m.actual_bounding_box_descent,
            INK_TOLERANCE,
        ),
    ];
    let mut max_dev = [0.0f64; 7];
    let mut failures = Vec::new();
    for case in cases {
        let text = case["text"].as_str().expect("text");
        let ours = case["ours"].as_str().expect("ours");
        let m = measurer.measure(text, ours).expect("measure");
        for (i, (name, field, tol)) in fields.iter().enumerate() {
            let want = get(case, name);
            let got = field(&m);
            let dev = (got - want).abs();
            max_dev[i] = max_dev[i].max(dev);
            if dev > *tol + 1e-9 {
                failures.push(format!(
                    "{text:?} in {ours:?}: {name} = {got}, Chrome {want} (off by {dev:.4}, tolerance {tol})"
                ));
            }
        }
    }
    println!("max deviation from Chrome over {} cases:", cases.len());
    for (i, (name, _, tol)) in fields.iter().enumerate() {
        println!("  {name:<28} {:.4} px (tolerance {tol})", max_dev[i]);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn deterministic_bits_across_measurers_and_order() {
    let texts = [
        ("Hello, world", "12px 'Noto Sans'"),
        ("売上高（百万円）", "bold 12px 'Noto Sans JP'"),
        ("Sales 売上 2026年", "13.5px sans-serif"),
        ("x", "italic 40px serif"),
    ];
    let a = TextMeasurer::new(common::test_fonts());
    let first: Vec<TextMetrics> = texts
        .iter()
        .map(|(t, f)| a.measure(t, f).unwrap())
        .collect();
    // Again on the same measurer (cached), and on a fresh one in reverse order.
    for (i, (t, f)) in texts.iter().enumerate() {
        assert_eq!(bits(&a.measure(t, f).unwrap()), bits(&first[i]));
    }
    let b = TextMeasurer::new(common::test_fonts());
    for (i, (t, f)) in texts.iter().enumerate().rev() {
        assert_eq!(bits(&b.measure(t, f).unwrap()), bits(&first[i]), "{t}");
    }
}

fn bits(m: &TextMetrics) -> [u64; 7] {
    [
        m.width.to_bits(),
        m.actual_bounding_box_left.to_bits(),
        m.actual_bounding_box_right.to_bits(),
        m.actual_bounding_box_ascent.to_bits(),
        m.actual_bounding_box_descent.to_bits(),
        m.font_bounding_box_ascent.to_bits(),
        m.font_bounding_box_descent.to_bits(),
    ]
}

#[test]
fn measures_identically_from_many_threads() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let want = measurer
        .measure("Revenue 2026", "bold 14px sans-serif")
        .unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let m = measurer.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    let got = m.measure("Revenue 2026", "bold 14px sans-serif").unwrap();
                    assert_eq!(bits(&got), bits(&want));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
}

#[test]
fn bad_font_strings_mean_canvas_default() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let default = measurer.measure("Label", "10px sans-serif").unwrap();
    for bad in [
        "",
        "garbage",
        "12px",
        "bold bold 12px x",
        "-3px x",
        "12px inherit",
    ] {
        assert_eq!(measurer.measure("Label", bad).unwrap(), default, "{bad:?}");
    }
}

#[test]
fn generic_families_follow_the_mapping() {
    let mut fonts = common::test_fonts();
    assert_eq!(fonts.families(), vec!["Noto Sans", "Noto Sans JP"]);
    assert_eq!(fonts.face_count(), 4);
    let before = TextMeasurer::new(fonts.clone());
    let latin = before.measure("Axis", "12px 'Noto Sans'").unwrap();
    let jp = before.measure("Axis", "12px 'Noto Sans JP'").unwrap();
    assert_ne!(latin.width, jp.width, "the two families differ for Latin");
    // Default: every generic maps to the first registered family.
    assert_eq!(before.measure("Axis", "12px monospace").unwrap(), latin);
    // Remapped, case-insensitively.
    fonts
        .set_generic_family(GenericFamily::Monospace, &["noto sans jp"])
        .unwrap();
    let after = TextMeasurer::new(fonts);
    assert_eq!(after.measure("Axis", "12px monospace").unwrap(), jp);
    assert_eq!(after.measure("Axis", "12px sans-serif").unwrap(), latin);
}

#[test]
fn japanese_falls_back_to_a_registered_cjk_font() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let explicit = measurer
        .measure("Sales 売上", "12px 'Noto Sans', 'Noto Sans JP'")
        .unwrap();
    for font in ["12px 'Noto Sans'", "12px sans-serif", "12px Unknown"] {
        assert_eq!(
            measurer.measure("Sales 売上", font).unwrap(),
            explicit,
            "{font}"
        );
    }
    // Two full-width ideographs at 12px are 24px wide in Noto Sans JP.
    let ideographs = measurer.measure("売上", "12px 'Noto Sans'").unwrap();
    assert!(
        (ideographs.width - 24.0).abs() < 1e-3,
        "{}",
        ideographs.width
    );
}

#[test]
fn whitespace_and_empty_text() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let spaces = measurer.measure("a b c d", "12px sans-serif").unwrap();
    let mixed = measurer.measure("a\tb\nc\rd", "12px sans-serif").unwrap();
    assert_eq!(spaces, mixed);
    let empty = measurer.measure("", "12px sans-serif").unwrap();
    assert_eq!(empty.width, 0.0);
    assert_eq!(empty.actual_bounding_box_right, 0.0);
    assert_eq!(empty.font_bounding_box_ascent, 13.0);
    let blank = measurer.measure("   ", "12px sans-serif").unwrap();
    assert!(blank.width > 0.0);
    assert_eq!(blank.actual_bounding_box_ascent, 0.0);
}

#[test]
fn hostile_inputs_do_not_panic() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let long_label = "長".repeat(10_000);
    let inputs: Vec<(&str, &str)> = vec![
        ("\u{0}\u{1}\u{fffd}", "12px sans-serif"),
        ("\u{202e}RTL override\u{202c}", "12px sans-serif"),
        ("مرحبا", "12px sans-serif"),
        ("😀👍🏽", "12px sans-serif"),
        ("a\u{2028}b\u{2029}c", "12px sans-serif"),
        ("e\u{301}\u{301}\u{301}", "12px sans-serif"),
        ("huge", "10000px sans-serif"),
        ("huger", "1e30px sans-serif"),
        ("zero", "0px sans-serif"),
        (&long_label, "12px sans-serif"),
    ];
    for (text, font) in inputs {
        let m = measurer.measure(text, font).unwrap();
        for v in bits(&m) {
            assert!(f64::from_bits(v).is_finite(), "{text:?} {font:?}: {m:?}");
        }
    }
    let zero = measurer.measure("zero", "0px sans-serif").unwrap();
    assert_eq!(zero.width, 0.0);
    let clamped = measurer.measure("huger", "1e30px sans-serif").unwrap();
    let max = measurer.measure("huger", "10000px sans-serif").unwrap();
    assert_eq!(clamped, max);
}

#[test]
fn errors() {
    assert_eq!(
        TextMeasurer::new(FontSet::new()).measure("x", "12px sans-serif"),
        Err(GfxError::NoFonts)
    );
    let mut fonts = FontSet::new();
    assert!(matches!(
        fonts.add_font_bytes(b"not a font".to_vec()),
        Err(GfxError::InvalidFont { .. })
    ));
    // A truncated real font is rejected or registered, but never panics.
    let mut bytes = common::font_bytes("NotoSans-Regular.ttf");
    bytes.truncate(2000);
    let _ = fonts.add_font_bytes(bytes);
}

#[test]
fn font_files_are_returned_unchanged() {
    let fonts = common::test_fonts();
    let files: Vec<&[u8]> = fonts.font_files().collect();
    assert_eq!(files.len(), 4);
    for (got, name) in files.iter().zip(common::FONT_FILES) {
        assert_eq!(*got, common::font_bytes(name).as_slice());
    }
}

/// Prints the cost of a call. Run in release for real numbers:
/// `cargo test --release -p surimono-gfx --test measure_text timing -- --nocapture`
#[test]
fn timing_short_labels() {
    let measurer = TextMeasurer::new(common::test_fonts());
    let labels: Vec<String> = (0..2000).map(|i| format!("{} 万円", i * 37)).collect();
    // Warm up glyph caches.
    for l in &labels {
        measurer.measure(l, "12px sans-serif").unwrap();
    }
    let start = std::time::Instant::now();
    let mut n = 0;
    for (i, l) in labels.iter().enumerate() {
        // A new font string per call defeats the result cache: real shaping.
        let font = format!("{}px sans-serif", 10 + (i % 7));
        measurer.measure(&format!("{l}x"), &font).unwrap();
        n += 1;
    }
    let shaped = start.elapsed() / n;
    let start = std::time::Instant::now();
    for l in &labels {
        measurer.measure(l, "12px sans-serif").unwrap();
    }
    let cached = start.elapsed() / labels.len() as u32;
    println!("measureText: {shaped:?} per call shaped, {cached:?} per call cached");
}
