//! Measured text equals laid-out text: Blitz 0.2.4, set up exactly as fulgur
//! 0.40.0 sets it up, gives a text box the width `TextMeasurer` measured.
//!
//! fulgur 0.40.0 `src/blitz_adapter.rs:303-348` (`parse_inner`) builds a
//! fontique collection with `system_fonts` as configured, registers every
//! `AssetBundle` font with `register_fonts(blob, None)`, hands it to
//! `HtmlDocument::from_html` as `DocumentConfig::font_ctx`, then lays out with
//! `doc.resolve(0.0)` (line 375). This test does the same with the same fonts.

mod common;

use blitz_dom::{DocumentConfig, FontContext};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use surimono_gfx::{FontSet, TextMeasurer};

/// Lays out `text` in an inline-block span. Returns the advance of its text
/// (the Parley line Blitz built), the span's unrounded box width, and the
/// box width after Taffy's pixel snapping.
fn blitz_layout(fonts: &FontSet, css_font: &str, text: &str) -> (f32, f32, f32) {
    let mut ctx = FontContext {
        collection: Collection::new(CollectionOptions {
            system_fonts: false,
            ..Default::default()
        }),
        source_cache: SourceCache::new(Default::default()),
    };
    for file in fonts.font_files() {
        let blob: Blob<u8> = file.to_vec().into();
        ctx.collection.register_fonts(blob, None);
    }
    let html = format!(
        "<!doctype html><html><body style=\"margin:0\"><span id=\"t\" \
         style=\"display:inline-block;white-space:pre;font:{css_font}\">{text}</span></body></html>"
    );
    let mut doc = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            viewport: Some(Viewport::new(4000, 1000, 1.0, ColorScheme::Light)),
            font_ctx: Some(ctx),
            base_url: Some("file:///".to_string()),
            ..DocumentConfig::default()
        },
    );
    doc.resolve(0.0);
    let id = doc.get_element_by_id("t").expect("span");
    let node = doc.get_node(id).expect("node");
    let advance = node
        .element_data()
        .and_then(|e| e.inline_layout_data.as_ref())
        .map(|t| t.layout.lines().map(|l| l.metrics().advance).sum())
        .unwrap_or(0.0);
    (
        advance,
        node.unrounded_layout.size.width,
        node.final_layout.size.width,
    )
}

fn blitz_width(fonts: &FontSet, css_font: &str, text: &str) -> f32 {
    blitz_layout(fonts, css_font, text).0
}

#[test]
fn measured_width_equals_blitz_layout_width() {
    let fonts = common::test_fonts();
    let measurer = TextMeasurer::new(fonts.clone());
    let stack = "'Noto Sans', 'Noto Sans JP'";
    let mut max_dev = 0.0f64;
    for (font, text) in [
        (format!("12px {stack}"), "Hello, world"),
        (format!("bold 14px {stack}"), "Revenue 2026"),
        (format!("13.5px {stack}"), "AVATAR Wave Toyota"),
        (format!("12px {stack}"), "売上高（百万円）"),
        (format!("bold 16px {stack}"), "Sales 売上 2026年"),
        (format!("11px {stack}"), "0123456789"),
        (format!("20px {stack}"), "office affine"),
        (
            "12px 'Noto Sans JP', 'Noto Sans'".to_string(),
            "Q1 Q2 Q3 Q4",
        ),
    ] {
        let measured = measurer.measure(text, &font).unwrap().width;
        let (advance, unrounded, snapped) = blitz_layout(&fonts, &font, text);
        println!(
            "{text:?} in {font}: measured {measured:.4}, Blitz text {advance:.4}, box {unrounded:.4} (snapped {snapped})"
        );
        // The glyphs Blitz lays out (and fulgur prints) advance exactly as
        // measured: same fonts, same Parley.
        let dev = (measured - advance as f64).abs();
        max_dev = max_dev.max(dev);
        assert!(
            dev <= 0.001,
            "{text:?} in {font}: measured {measured}, Blitz {advance}"
        );
        // A shrink-to-fit box around them is the advance rounded up (blitz-dom
        // 0.2.4 src/layout/inline.rs:90), then snapped to whole px by Taffy.
        assert_eq!(unrounded, advance.ceil(), "{text:?}");
        assert_eq!(snapped, unrounded.round(), "{text:?}");
    }
    println!("max deviation from Blitz layout: {max_dev:.4} px");
}

/// With system fonts off, fontique has no generic families, so Blitz finds no
/// font for `sans-serif` and lays the text out with zero width. This is why
/// sanitize_svg writes registered family names, never generics.
#[test]
fn generic_families_find_no_font_in_blitz_without_system_fonts() {
    let fonts = common::test_fonts();
    assert_eq!(blitz_width(&fonts, "12px sans-serif", "Label"), 0.0);
    assert!(blitz_width(&fonts, "12px 'Noto Sans'", "Label") > 20.0);
}
