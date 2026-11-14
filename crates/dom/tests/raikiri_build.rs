//! E3-R1 stage A: the Raikiri crates build and link under the `raikiri`
//! feature, with nothing using them yet. Run with
//! `cargo test -p surimono-dom --features raikiri --test raikiri_build`.
//!
//! What it holds the build to (crates/dom/RAIKIRI.md, sections 4 and 8): a
//! font set made from the caller's bytes alone, with the installed fonts off,
//! handed to a document Raikiri lays out; and, checked on this test's binary
//! with `ldd` (DECISIONS, E3-R1 stage A), no load-time dependency on
//! libfontconfig, which fontique's `fontconfig-dlopen` feature defers to a
//! request for system fonts that surimono never makes.
#![cfg(feature = "raikiri")]

use raikiri_html::FontCollectionBuilder;

fn font_file(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn the_fixture_fonts_alone_make_a_font_set_for_a_raikiri_document() {
    let fonts = FontCollectionBuilder::new()
        .system_fonts(false)
        .font_bytes("Noto Sans", font_file("NotoSans-Regular.ttf"))
        .font_bytes("Noto Sans", font_file("NotoSans-Bold.ttf"))
        .build()
        .expect("the fixture fonts make a font set");
    // Only the bundle answers a lookup, so no answer depends on the machine.
    assert!(fonts.is_bundled_only());

    let mut doc = raikiri_dom::Document::new();
    // Fonts first, then the single-threaded paragraph build, which a font
    // set turns back on (RAIKIRI.md, "Threads").
    doc.set_font_collection(fonts.collection().clone());
    doc.set_ifc_parallel_build(false);
    // A new document is its document node alone.
    assert_eq!(doc.node_count(), 1);
}

#[test]
fn no_font_set_is_made_from_no_fonts() {
    // Raikiri refuses an empty bundle instead of falling back to anything.
    assert!(
        FontCollectionBuilder::new()
            .system_fonts(false)
            .build()
            .is_err()
    );
}
