# Handing fonts to fulgur

How the caller makes the PDF print text exactly as surimono measured it. Line
numbers refer to fulgur 0.40.0 (crate source from static.crates.io),
blitz-dom 0.2.4, Parley 0.6.0, fontique 0.6.0 and the fulgur Ruby binding at
tag v0.40.0 (`crates/fulgur-ruby/`).

fulgur prints text along two different paths, and only one of them uses the
fonts you give it:

| Text | How fulgur 0.40.0 prints it | Fonts | What surimono does |
|---|---|---|---|
| HTML text (the page) | Blitz lays it out with Parley | your `AssetBundle` fonts | nothing; hand fulgur the same font files (below) |
| Inline `<svg>` text (every island chart) | blitz-dom parses the SVG with usvg, fulgur draws that tree | **system fonts only** | outlines island text with the registered fonts (`TextMode::Outlines`, the default) |

## HTML text: the same fonts, the same shaper

Measured text equals printed HTML text when the renderer has:

1. **the same font files**: the bytes given to `FontSet::add_font_bytes` also
   go to fulgur's `AssetBundle`;
2. **the same shaper**: surimono-gfx pins `parley =0.6.0`, the version
   blitz-dom 0.2.4 uses and fulgur 0.40.0 prints with. Bump them together;
3. **named families**: the page's CSS names registered families (see
   "Generic families" below);
4. **no other fonts**: turn system fonts off in fulgur, or at least make the
   registered fonts cover every character (see "Ruby" below).

Then the glyphs fulgur lays out advance exactly as measured.
`tests/fulgur_layout.rs` proves it: it sets up Blitz the way fulgur does and
compares the laid-out text advance with `TextMeasurer::measure` for Latin,
bold, Japanese and mixed strings. Maximum difference: **0.0000 px**.

## Island SVG text: outlined, because fulgur uses system fonts there

fulgur depends on blitz-dom with its default features, which include `svg`.
blitz-dom parses every inline `<svg>` with usvg while it builds the DOM
(`src/layout/construct.rs:112-134` → `src/util.rs:104-110`), and gives usvg a
static font database loaded **from system fonts only** (`src/util.rs:12-16`,
`load_system_fonts()`). fulgur prints that tree as it is
(`src/blitz_adapter.rs:1142-1148` `extract_inline_svg_tree`,
`src/convert/replaced.rs:222-231`). Nothing passes the `AssetBundle` fonts to
usvg, and `system_fonts(false)` never reaches that database. So island labels
would print in whatever fonts the server has (`WenQuanYiZenHei` on the build
machine), or not at all where none covers them, and they would no longer fit
the space measured for them.

So `sanitize_svg` replaces every SVG `<text>` with `<path>` outlines of its
glyphs, shaped by the same Parley code as `TextMeasurer::measure` from the
registered fonts. Any PDF engine then prints exactly what was measured, with
no font lookup at all. A `<text>` becomes a `<g>` that keeps its id, class,
paint and transform, so the PDF keeps the label's meaning for assistive
technology, but the label can no longer be selected, searched or copied in
the PDF. The group carries:

- `data-surimono-text`: the string that was outlined, always (after white
  space collapsing). The conformance text checks read it first; Chrome's
  rendering of the original `<text>` stays the expectation.
- `data-surimono-face`: the faces Parley shaped it with, in order of first
  use, by their name-table family and style, e.g. `Noto Sans JP Bold, Noto
  Sans Bold` for bold mixed Japanese and Latin. `(synthetic oblique)` marks
  a slanted face standing in for an italic, and a variable face adds its
  normalized coordinates (`[wght 0.5]`). The checks can verify the face
  really used, not only the declared weight. Same input, same string.
- The author's `aria-label` and `role` if it had them (Vega's text marks
  do), otherwise `role="img"` and `aria-label` set to the outlined string.

Any `data-surimono-*` attribute in the input is dropped first, so an island
can't forge them. Each glyph path carries `style="fill-rule:nonzero!important"`
(in a `<clipPath>`, `clip-rule`), so neither the island nor the page can make
overlapping glyphs cancel out.

Evidence (all in `crates/gfx/tests/`):

- `fulgur_print.rs` prints islands with the fulgur CLI 0.40.0 and reads the PDF
  with `pdffonts`. Outlined: the only font in the PDF is the registered one of
  the page's HTML label. Kept: `WenQuanYiZenHei`, a system font. The kept case
  is an **expected failure**, recorded in `docs/COMPAT.md`; the test asserts it
  still happens, so a fulgur or Blitz fix shows up as a failing test.
- `svg::text::tests::layout_matches_chrome` checks the outline layout against
  where Chromium 153 puts every character of 20 SVG text cases
  (`chrome/svg_text.mjs`): 523 characters within 0.049 px, baselines within
  0.0001 px.
- `chrome/svg_outline.mjs` renders real ECharts and Vega output in Chrome as
  outlines and as Chrome's own text with the same fonts: 0.00%, 0.00% and
  0.32% of ink pixels differ (a system-font control: 2.8% to 7.7%).

### Opting out: `TextMode::Keep`

```rust
use surimono_gfx::{SvgOptions, TextMode, sanitize_svg};
let svg = sanitize_svg(raw, island, &measurer, &SvgOptions::new().with_text(TextMode::Keep))?;
```

Keep leaves `<text>` as text, with every `font-family` rewritten to the
registered families measurement used. Use it only with a renderer that lays
out inline SVG text with your fonts: a browser that loads them with
`@font-face` (the Chrome references do), or a fulgur whose Blitz builds the
usvg font database from the document's fonts (asked upstream, see
`docs/upstream/blitz-svg-document-fonts.md`). With fulgur 0.40.0, Keep means
island labels print in system fonts: different glyphs, different widths, and
nothing at all for characters the server's fonts lack. It also puts system
fonts in the render path, which breaks "same input, same output" across
machines.

### What outlining does not cover

The outline layout follows Chrome for what chart libraries emit: white-space
collapsing, `x`/`y`/`dx`/`dy` lists, chunks and `text-anchor`,
`letter-spacing`, `dominant-baseline` and `alignment-baseline`, `font`,
`font-family`, `font-size`, `font-weight` and `font-style` from attributes and
`style`, `<tspan>` and `<a>`. Not supported: `rotate`, `textLength`,
`<textPath>` (its content is dropped), vertical writing modes,
`word-spacing`, `baseline-shift`, text decoration, synthetic bold, and fonts
inherited from the page outside the SVG (an island's text with no font
settings uses 16px and the `serif` mapping). A `<text>` inside `<symbol>` or
`<defs>` takes its inherited font from where it is defined, not from the
`<use>` that shows it. ECharts and Vega use none of these. (`<style>` never
reaches outlining: the sanitiser removes it, see below.)

Outlining runs on the host after the island's script limits have ended, so
it has bounds of its own. Going over one is `GfxError::InvalidSvg`:

- at most `MAX_OUTLINE_CHARS` (20,000) shaped characters per SVG; every
  character that is shaped counts, including preserved spaces, U+00A0 and
  U+3000, and collection stops at the limit;
- at most 20,000 `<text>` elements per SVG, empty ones included;
- at most 32 MiB of path data, checked as each path command is written, so
  memory peaks at the budget plus one command;
- each element's style parsed once, top down, and CSS over 64 KiB in one
  attribute isn't read (the sanitiser drops it);
- glyphs whose origin is more than 10,000,000 px from the text's origin are
  not drawn (no page shows them), and lengths are clamped to 1e9, so every
  path number is at most 12 bytes.

The review's hostile cases now take milliseconds, release build: 1,000
characters at `x=1e300` took 31 s and 208 MB before and take 1 ms and 15 MB
now; a 1 MB ancestor style around 1,000 empty texts took 72.5 s and takes
38 ms now; 6.4 MB of U+00A0 took 4.2 s and is refused in 52 ms.
`GfxError::TextTooLong` can't come out of `sanitize_svg`.

### What the sanitiser removes so an island stays in its box

An inlined island can't run or load anything, and can't affect anything
outside its own box. Beyond scripts, handlers and external URLs:

- every `<style>` (its rules would apply to the whole page), all of
  `<foreignObject>`, and all SMIL animation;
- every element inside `<desc>` and `<title>` (an HTML parser reads them as
  HTML, where `<textarea/>` swallows the rest of the page and `<body>`
  restyles the page's own body); their text stays;
- on the root `<svg>`, everything but size, `viewBox`,
  `preserveAspectRatio`, `version`, `baseProfile`, `class`, `id`, `role`,
  ARIA, language and inherited paint and font settings, in attributes and
  in `style` (no `position`, `transform`, `overflow`, `filter`, `display`);
  and a prefixed root (`<svg:svg>`) is rejected.

All CSS that stays (`style` attributes, `fill`, `mask`, ...) is read with a
CSS Syntax Level 3 tokenizer and checked again after rewriting; anything
that could load (`image-set()`, `src()`, ...) goes. `tests/svg.rs`
(`islands_stay_in_their_box`) parses the page around each sanitized island
with html5ever, as fulgur does, and `chrome/svg_inline.mjs` checks the same
in Chrome.

## Rust

```rust
use surimono_gfx::{FontSet, TextMeasurer};

let mut fonts = FontSet::new();
let mut bundle = fulgur::AssetBundle::new();
for path in ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf", "NotoSansJP-Regular.otf"] {
    let bytes = std::fs::read(path)?;
    fonts.add_font_bytes(bytes.clone())?; // measured (and outlined) with these
    bundle.add_font_bytes(bytes)?;        // HTML text printed with these
}
let measurer = TextMeasurer::new(fonts);
// ... run the islands with `measurer`, then:
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
| Inline SVG drawn from blitz-dom's usvg tree | `src/blitz_adapter.rs:1142-1148`, `src/convert/replaced.rs:222-231` |

And in Blitz and Parley:

| What | Where |
|---|---|
| CSS `font-family` becomes a Parley `FontStack::List`; names stay names, generics become `GenericFamily` | blitz-dom `src/stylo_to_parley.rs:133-158`, `:176` |
| Text is built with `quantize = true`: line ascent and descent are rounded to whole px, as `TextMetrics::font_bounding_box_*` are | blitz-dom `src/layout/construct.rs:739`, Parley `src/layout/line/greedy.rs:608` |
| A generic family resolves through the collection's generic map, which is empty without system fonts | fontique `src/collection/query.rs:71`, `src/collection/mod.rs:315` |
| Inline content widths are rounded **up** to whole px | blitz-dom `src/layout/inline.rs:90`, `:128`, `:257` |
| Inline `<svg>` parsed with usvg, font database from system fonts only | blitz-dom `src/layout/construct.rs:112-134`, `src/util.rs:12-16`, `:104-110` |

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

Island charts print the same either way, because their text is outlines.
For the page's HTML text, system fonts stay on in the gem, but fontique looks
up a named family in the registered fonts before the system's
(fontique `src/collection/mod.rs:256`, `Inner::family_id`). So a page that
names registered families prints with them, and only characters no registered
font has fall back to a system font.

## Things that differ, and what to do

- **Generic families in your own HTML.** With system fonts off, fulgur finds
  no font for `sans-serif`, `serif` or `monospace`, and that text lays out
  with zero width (`tests/fulgur_layout.rs` shows it). In the page's own CSS,
  name a registered family. Island SVG is not affected (it is outlined).
- **Island text is not selectable.** Outlined labels are paths: a PDF reader
  can't search or copy them. The string stays in `data-surimono-text` (and
  in `aria-label` unless the chart set its own).
- **WOFF and WOFF2.** fulgur decodes WOFF2; surimono-gfx rejects both. Give
  both the TTF or OTF.
- **Boxes around text.** Blitz rounds a shrink-to-fit inline box up to whole
  px (Chrome keeps 1/64 px), so an `inline-block` label is up to 1 px wider
  than its text. Glyph positions are not rounded.
- **Upgrades.** A fulgur release that moves to Blitz 0.3 (Parley 0.7) needs
  the pin in `crates/gfx/Cargo.toml` moved in the same change, and
  `tests/fulgur_layout.rs` and `tests/fulgur_print.rs` rerun.

## Worth asking upstream (for Kay)

- blitz-dom: build the inline-SVG font database from the document's fonts
  (or `DocumentConfig`) instead of system fonts. Draft:
  `docs/upstream/blitz-svg-document-fonts.md`. With that in a fulgur release,
  `TextMode::Keep` becomes safe and labels become selectable again.
- fulgur-ruby: expose `system_fonts:` on `Engine.new` and `add_font_bytes`
  on `AssetBundle`, as the Rust API has.
- fulgur: a way to map generic families to bundled fonts (fontique's
  `Collection::set_generic_families`), so `font-family: sans-serif` prints
  with system fonts off.
