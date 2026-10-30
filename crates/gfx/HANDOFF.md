# Handing fonts to fulgur

How the caller makes the PDF print text exactly as surimono measured it. Line
numbers refer to fulgur 0.40.0 (crate source from static.crates.io),
blitz-dom 0.2.4, Parley 0.6.0, fontique 0.6.0 and the fulgur Ruby binding at
tag v0.40.0 (`crates/fulgur-ruby/`).

## The rule

Measured text equals printed text when the renderer has:

1. **the same font files**: the bytes given to `FontSet::add_font_bytes` also
   go to fulgur's `AssetBundle`;
2. **the same shaper**: surimono-gfx pins `parley =0.6.0`, the version
   blitz-dom 0.2.4 uses and fulgur 0.40.0 prints with. Bump them together;
3. **the same family stack**: `sanitize_svg` rewrites every `font-family` in
   island SVG to `FontSet::resolve_families`, the registered families
   `measureText` used, in the same order, so fulgur gets named families only;
4. **no other fonts**: turn system fonts off in fulgur, or at least make the
   registered fonts cover every character (see "Ruby" below).

Then the glyphs fulgur lays out advance exactly as measured.
`tests/fulgur_layout.rs` proves it: it sets up Blitz the way fulgur does and
compares the laid-out text advance with `TextMeasurer::measure` for Latin,
bold, Japanese and mixed strings. Maximum difference: **0.0000 px**.

## Rust

```rust
use surimono_gfx::FontSet;

let mut fonts = FontSet::new();
let mut bundle = fulgur::AssetBundle::new();
for path in ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf", "NotoSansJP-Regular.otf"] {
    let bytes = std::fs::read(path)?;
    fonts.add_font_bytes(bytes.clone())?; // measured with these
    bundle.add_font_bytes(bytes)?;        // printed with these
}
// ... run the islands with `fonts`, then:
let engine = fulgur::Engine::builder()
    .assets(bundle)
    .system_fonts(false)
    .build();
```

`FontSet::font_files()` returns the registered bytes unchanged, for callers
that only keep the `FontSet`.

Where this lands in fulgur:

| What | Where |
|---|---|
| `AssetBundle::new`, `add_font_file`, `add_font_bytes` | `src/asset.rs:19`, `:38`, `:65` |
| WOFF2 is decoded, WOFF1 rejected, unknown formats passed through | `src/asset.rs:66-77` |
| `EngineBuilder::assets`, `system_fonts` | `src/engine.rs:1319`, `:1324` |
| System fonts default to **on** | `src/engine.rs:84` |
| Fonts and the system-font flag go to the parser | `src/engine.rs:103`, `:197-204` |
| No fonts and system fonts on: Blitz's default font context | `src/blitz_adapter.rs:319-322` |
| Otherwise a fontique `Collection` with `system_fonts` as set, every font registered with `register_fonts(blob, None)` | `src/blitz_adapter.rs:324-336` |
| Layout | `src/blitz_adapter.rs:375` (`doc.resolve(0.0)`) |

And in Blitz and Parley:

| What | Where |
|---|---|
| CSS `font-family` becomes a Parley `FontStack::List`; names stay names, generics become `GenericFamily` | blitz-dom `src/stylo_to_parley.rs:133-158`, `:176` |
| Text is built with `quantize = true`: line ascent and descent are rounded to whole px, as `TextMetrics::font_bounding_box_*` are | blitz-dom `src/layout/construct.rs:739`, Parley `src/layout/line/greedy.rs:608` |
| A generic family resolves through the collection's generic map, which is empty without system fonts | fontique `src/collection/query.rs:71`, `src/collection/mod.rs:315` |
| Inline content widths are rounded **up** to whole px | blitz-dom `src/layout/inline.rs:90`, `:128`, `:257` |

## Ruby

fulgur's gem (v0.40.0) only takes font files and has no switch for system
fonts: `AssetBundle#add_font_file` (`crates/fulgur-ruby/src/asset_bundle.rs:45`,
`:74`); `Engine.new` accepts `page_size`, `margin`, `landscape`, `title`,
`author`, `lang`, `bookmarks` and `assets` (`crates/fulgur-ruby/src/engine.rs:213-243`).
So pass the same paths to both:

```ruby
FONTS = %w[NotoSans-Regular.ttf NotoSans-Bold.ttf NotoSansJP-Regular.otf]

html = Surimono.render(template_html, fonts: FONTS)   # API per P1-PKG-04
bundle = Fulgur::AssetBundle.new
FONTS.each { |path| bundle.add_font_file(path) }
pdf = Fulgur::Engine.new(page_size: :a4, assets: bundle).render_html(html)
```

With system fonts on, island text still prints as measured: fontique looks up
a named family in the registered fonts before the system's
(fontique `src/collection/mod.rs:256`, `Inner::family_id`), and the rewritten stack lists
every registered family, so system fallback is only reached for characters no
registered font has. Those print in a system font but measure as the
first family's missing-glyph box.

## Things that differ, and what to do

- **Generic families in your own HTML.** With system fonts off, fulgur finds
  no font for `sans-serif`, `serif` or `monospace`, and that text lays out
  with zero width (`tests/fulgur_layout.rs` shows it). Island SVG is safe
  because `sanitize_svg` writes registered names. In the page's own CSS, name
  a registered family.
- **WOFF and WOFF2.** fulgur decodes WOFF2; surimono-gfx rejects both. Give
  both the TTF or OTF.
- **Boxes around text.** Blitz rounds a shrink-to-fit inline box up to whole
  px (Chrome keeps 1/64 px), so an `inline-block` label is up to 1 px wider
  than its text. Glyph positions are not rounded.
- **Upgrades.** A fulgur release that moves to Blitz 0.3 (Parley 0.7) needs
  the pin in `crates/gfx/Cargo.toml` moved in the same change, and
  `tests/fulgur_layout.rs` rerun.

## Worth asking upstream (for Kay)

- fulgur-ruby: expose `system_fonts:` on `Engine.new` and `add_font_bytes`
  on `AssetBundle`, as the Rust API has.
- fulgur: a way to map generic families to bundled fonts (fontique's
  `Collection::set_generic_families`), so `font-family: sans-serif` prints
  with system fonts off.
