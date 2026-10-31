# blitz-dom: give inline SVG the document's fonts, not system fonts

**Project:** blitz-dom 0.2.4 (as used by fulgur 0.40.0) · **Status:** draft for Kay to file · **Local patch:** none (see "Why no local patch")

## Problem
With the `svg` feature (on by default, and on in fulgur), blitz-dom parses
every inline `<svg>` with usvg while it constructs the DOM, and passes usvg a
process-wide font database that loads **system fonts only**:

- `src/util.rs:12-16`: `static FONT_DB` = `fontdb::Database::new()` +
  `load_system_fonts()`;
- `src/util.rs:104-110`: `parse_svg` builds `usvg::Options { fontdb: FONT_DB, .. }`;
- `src/layout/construct.rs:112-134`: inline `<svg>` → `parse_svg(outer_html)`.

The fonts the embedder configured (`DocumentConfig::font_ctx`, a fontique
collection) never reach usvg. So `<text>` inside inline SVG ignores the
document's fonts: with fulgur, an `AssetBundle` font and `system_fonts(false)`
apply to HTML text but not to SVG text. SVG text prints in whatever fonts the
machine has (output differs between machines), or not at all where none
covers the characters.

## Repro
fulgur-cli 0.40.0 on Linux, fonts from notofonts:

```html
<!doctype html><meta charset="utf-8">
<p style="font:14px 'Noto Sans JP'">HTML 売上高 Revenue</p>
<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60">
  <text x="0" y="20" font-family="'Noto Sans', 'Noto Sans JP'" font-size="14">SVG 売上高 Revenue</text>
</svg>
```

```sh
fulgur render -f NotoSans-Regular.ttf -f NotoSansJP-Regular.otf -o out.pdf in.html
pdffonts out.pdf
```

Expected: only the Noto fonts. Actual: `NotoSansJP-Regular` for the `<p>` and
`WenQuanYiZenHei` (a system font) for the SVG `<text>`, although the SVG names
Noto Sans, which the document has.

## Proposal (minimal patch)
Build the usvg font database from the document's fonts when the document has
any:

1. Keep a `fontdb::Database` on `BaseDocument`, filled when fonts are
   registered: for each font blob in the document's fontique collection (or
   each blob passed through `DocumentConfig`), `db.load_font_source(
   fontdb::Source::Binary(Arc::new(blob)))`. Load system fonts into it only
   when the collection has system fonts enabled (that is what fulgur's
   `system_fonts(false)` means).
2. Change `parse_svg(source)` to `parse_svg(source, fontdb: Arc<fontdb::Database>)`
   and pass the document's database from `construct.rs`.
3. Optionally set usvg's `font_family` default to the first registered
   family, so SVG text with no `font-family` doesn't fall back to "Times New
   Roman".

The static `FONT_DB` can stay as the fallback for documents built without a
font context, so nothing changes for current users of `DocumentConfig::default()`.

A second, smaller ask: expose the database (or a hook to add fonts to it), so
an embedder like fulgur can pass its `AssetBundle` fonts even if 1. is not
taken.

## Why no local patch
A `[patch]` of blitz-dom in surimono would not reach users: they print with the
fulgur gem or crate, which ships its own compiled Blitz. So surimono works
around it instead: island text is converted to vector outlines with the
registered fonts (`surimono_gfx::TextMode::Outlines`, the default since
P1-GFX-05). Once a fulgur release contains this fix, surimono can offer
`TextMode::Keep` (selectable text) as safe for fulgur.

## Test that shows it
`crates/gfx/tests/fulgur_print.rs`, `kept_island_text_prints_in_system_fonts_expected_failure`:
prints the repro with the fulgur CLI and asserts a system font still appears
(the known failure, recorded in `docs/COMPAT.md`). With the fix it fails,
which is the signal to update COMPAT.md and `crates/gfx/HANDOFF.md`.
