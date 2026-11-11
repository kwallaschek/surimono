# blitz-dom 0.2.4: print gaps seen through fulgur 0.40.0, with root causes

**Project:** blitz-dom 0.2.4 (the version fulgur 0.40.0 and 0.41.0 pin; with taffy 0.9.2, stylo 0.8.0, parley 0.6.0) · **Status:** drafts for Kay to file, one issue each · **Local patch:** none in surimono's workspace; B1, B2 and B3 were proved in a scratch build of fulgur-cli 0.40.0 (scores below)

Found by Q-GFX-01 (11 Nov 2026) while tracing why four of surimono's six real-world
pages print wrong in fulgur although their DOM equals Chrome's
(`fixtures/checkpoint-3/NOTES.md`). Each item has a few-line reproducer, what
Chromium 153 prints, the cause in the source, the smallest patch, and whether
Blitz 0.3.0-beta.4 (the newest on crates.io, 16 Oct 2026) already has it.
"Checked" means the published crate source was read, not run.

Most of these are already fixed in the 0.3 betas. fulgur 0.41.0 still pins
blitz-dom 0.2.4, so the fix reaches a fulgur user only when fulgur moves to 0.3
or when a 0.2.x point release carries a backport. The asks to Blitz are
therefore: B1 to B3 as a 0.2.5 backport (three small patches), B4 and B5 as
status questions for 0.3.

Reproducers assume `fulgur render --font NotoSans-Regular.ttf -o out.pdf in.html`
and `body { font: 14px/22px "Noto Sans" }` unless they say otherwise.

## B1. A block-level `<svg>` or `<img>` ignores its `width` and `height` attributes

```html
<style>svg { display: block }</style>
<svg width="100" height="50" viewBox="0 0 100 50"><rect width="100" height="50" fill="#2f6fb0"/></svg>
```

**Chrome:** a 100 × 50 box. CSS 2.2 §10.3.4: a block-level replaced element in
normal flow takes its width as an inline replaced element does (§10.3.2), so it
never stretches to the containing block. The attributes are the element's
`width`/`height`: SVG 2 "Sizing properties" makes them presentation attributes
on the outermost `<svg>`, and HTML "Attributes for embedded content and images"
maps them to the dimension properties for `<img>`.

**blitz-dom 0.2.4:** the box is as wide as the containing block (400 × 200 on a
400 px page) and the height follows from the aspect ratio. Page `d3-charts`
fails 100% for this alone (`d3.select(...).append("svg").attr("width", w)` with
`svg { display: block }`, a common D3 idiom).

**Root cause.** Two pieces meet:

- taffy 0.9.2 `src/compute/block.rs:424-437` stretch-sizes every in-flow block
  child whose style width is `auto`: `known_dimensions.width =
  Some(container_inner_width - margins)`. Its own comment says "TODO: Allow
  stretch-sizing to be conditional, as there are exceptions"; only tables are
  exempt (`is_table`).
- blitz-dom reads the attributes late, in the measure function only:
  `src/layout/mod.rs:218-262` builds `attr_size` and calls
  `replaced_measure_function`, and `src/layout/replaced.rs:90-94` returns
  `known_dimensions` (with the aspect ratio applied) before it looks at
  `attr_size`. `synthesize_presentational_hints_for_legacy_attributes`
  (`src/stylo.rs:887-915`) maps `width` for `table`, `col`, `tr`, `td`, `th`
  and `hr`, and `height` for `table`, `thead`, `tbody` and `tfoot`; neither for
  `svg` or `img`, so the style width stays `auto`.

An inline `<svg>` works because inline layout passes `known_dimensions: NONE`.

**Smallest fix** (blitz-dom `src/stylo.rs`, next to the table hints; 14 lines,
trivial, low risk: it makes the attribute a `PresHints`-level declaration, which
author CSS still overrides):

```diff
@@ fn synthesize_presentational_hints_for_legacy_attributes
+            if (*tag == local_name!("svg") || *tag == local_name!("img"))
+                && (*name == local_name!("width") || *name == local_name!("height"))
+            {
+                if let Some(size) = parse_size_attr(value, |v| *v >= 0.0) {
+                    use style::values::generics::{NonNegative, length::Size};
+                    let size = Size::LengthPercentage(NonNegative(size));
+                    push_style(if *name == local_name!("width") {
+                        PropertyDeclaration::Width(size)
+                    } else {
+                        PropertyDeclaration::Height(size)
+                    });
+                }
+            }
```

A replaced element with neither attribute nor CSS size (`<img>` at its natural
size, `display: block`) still stretches; that needs the taffy exception
(a "replaced" flag on `BlockItem` beside `is_table`), a day.

**Newer releases.** Fixed in 0.3.0-beta.4: `src/stylo.rs:1020-1118` maps the
dimension attributes of `img`, `iframe`, `embed`, `video`, `object` and, with
SVG length parsing, `svg`; `src/layout/replaced.rs` has a full
`compute_replaced_layout`. Not in 0.2.4.

**Workaround without touching the printer.** Conditional. Writing the
attributes as an inline style, `<svg width="640" height="208"
style="width:640px;height:208px">`, prints the same in Chrome only under these
conditions, because an inline style outranks every author rule while a
presentation attribute loses to all of them:

- per property: write `width` only when no author declaration of `width`
  applies to the element, and `height` only when none of `height` does. Any
  value counts, `auto`, `inherit`, `unset` and `revert` included, and so does
  `all`. Counter-example (Chrome 153): with the common reset `svg { display:
  block; max-width: 100%; height: auto }` in a 200 px box, the original prints
  200 × 65 px; with `style="width:640px;height:208px"` it prints a 200 × 208 px
  box with the drawing letterboxed in it, and the text below moves 143 px.
  Here `height` must not be written;
- only on an `<svg>` whose parent is not an SVG element, and on `<img>`;
- the value converted by the attribute's own rules: SVG lengths (`10em`,
  `100mm`, percentages) for `<svg>`, HTML's dimension rules for `<img>`.

Where a condition fails the element is left alone and prints as before the
workaround.

**Proof.** fulgur-cli 0.40.0 rebuilt with this patch alone prints `d3-charts`
at 0.0% / 0.0% / 0.0% (before: 100% / 100% / 100%); the workaround, with the
unpatched binary, gives the same. Method in `fulgur-layout-gaps-0.40.md`
("Scores").

## B2. A forced break at the end of a block adds an empty line

```html
<pre style="background:#eef2f6">one
two
</pre>
<div style="background:#eef2f6">one<br>two<br></div>
```

**Chrome:** both boxes are two lines high. CSS 2.2 §9.4.2: line boxes are
created as needed to hold inline-level content, and one with no text, no
preserved white space and no other in-flow content is treated as zero-height.
After the last forced break nothing is left to hold. (The HTML parser drops a
newline only directly after `<pre>`, not before `</pre>`, so the trailing one is
real text; Markdown renderers always emit it: `<pre><code>…\n</code></pre>`.)

**blitz-dom 0.2.4:** both boxes are three lines high.

**Root cause.** parley 0.6.0 `src/layout/line/greedy.rs:570-600` ends a layout
whose text ends in a newline with one more, empty line ("HACK: copy metrics
from previous line … an empty line following a newline at the end of a layout",
kept so a text cursor has a place). `Layout::height()` includes it, and
blitz-dom `src/layout/inline.rs:129` and `:258` return that height unchanged.
`<br>` is pushed as `"\n"` in preserve mode (`src/layout/construct.rs:863-869`),
so a trailing `<br>` does the same.

**Smallest fix** (blitz-dom `src/layout/inline.rs`; 15 lines, trivial, low
risk):

```diff
-                        height: layout.layout.height() / scale,
+                        height: css_height(&layout.layout, &layout.text) / scale,
 …
-                    height: inline_layout.layout.height() / scale,
+                    height: css_height(&inline_layout.layout, &inline_layout.text) / scale,
 …
+/// Parley ends a layout whose text ends in a forced break with one more,
+/// empty line. CSS has no line box there (CSS 2 §9.4.2).
+fn css_height<B: parley::Brush>(layout: &parley::Layout<B>, text: &str) -> f32 {
+    let height = layout.height();
+    if layout.len() > 1 && text.ends_with('\n') {
+        if let Some(last) = layout.get(layout.len() - 1) {
+            if last.text_range().is_empty() {
+                return height - last.metrics().line_height;
+            }
+        }
+    }
+    height
+}
```

An embedder that walks Parley's lines itself has to skip the same line.
fulgur does, in its fragmenter (`pagination_layout.rs:2323`,
`collect_inline_line_metrics`): with the block height fixed and that function
unchanged, the box after a body-level `<pre>` is still placed one line too low
(seen in the scratch build; three lines there fix it).

**Newer releases.** Fixed the same way in 0.3.0-beta.4
(`src/layout/inline.rs:845-862`: "A forced line break … at the end of the inline
content ends the final line box without starting a new one. Parley still emits
an empty line after it … so that line is excluded from the measured height").
Not in 0.2.4.

**Workaround without touching the printer.** Conditional: drop the last
forced break of a block. Two things have to be kept apart: whether fulgur then
prints what Chrome prints for the original, and whether the rewritten markup
still prints the same in Chrome (it has to, if the rewritten DOM is ever
compared with Chrome's). Block heights at 14px/22px, measured on prints of
each block, original and rewritten, by Chromium 153 and by fulgur-cli 0.40.0:

| Block | Chrome, original | Chrome, rewritten | fulgur, original | fulgur, rewritten |
|---|---|---|---|---|
| `one<br>` | 22 | 22 | 44 | 22 |
| `one<br><br>` | 44 | 22 | 66 | 44 |
| `<div><br></div>` | 22 | 0 | 44 | 0 |
| `<pre>one\n\n</pre>` | 44 | 22 | 64 | 44 |

The rule that follows from it:

- Drop the last forced break only when the text before it on its line is not
  empty (`one<br>`, `…two\n</pre>`). Then fulgur prints Chrome's height and
  the rewrite is neutral in Chrome.
- Two or more trailing breaks (`one<br><br>`, `one\n\n`): dropping one makes
  fulgur print Chrome's original height, but the rewritten markup is one line
  shorter in Chrome. Right as a form for fulgur only, not Chrome-neutral.
- A block whose only content is the `<br>` (the empty paragraph rich-text
  editors emit) is left alone: without the `<br>` it is 0 px high in both.
  fulgur keeps printing it one line too tall; there is no workaround.
- Not a `<br>` with `clear` (`<br clear="all">`, `br { clear: both }`): the
  block would stop containing its floats.
- Not when a selector can see the removal: `:last-child` on the sibling
  before it (`<p><b>x</b><br></p>`), `:empty`, `br` itself.
- The newline is preserved text under `white-space: pre`, `pre-wrap`,
  `pre-line` and `break-spaces`, so the text-node half applies to all four.

The cost in every case is that the printed DOM is no longer byte-equal to the
page's.

## B3. `@font-face` in a `<style>` element is never fetched; a fetched font keeps its own family name

```html
<style>
@font-face { font-family: "Noto Sans"; src: url("fonts/NotoSans-Regular.ttf") format("truetype") }
body { font: 20px "Noto Sans" }
</style>
<p>Hamburgefonstiv</p>
```

**Chrome:** loads the font and uses it (CSS Fonts 4 §4.3, the `src`
descriptor). The `font-family` descriptor is the name CSS matches, whatever
family name the font file carries (§4.2).

**blitz-dom 0.2.4:** the provider is never asked for the font. The same rule in
a linked stylesheet is fetched.

**Root cause.** `fetch_font_face` (`src/net.rs:252-306`) is called from two
places only: `CssHandler::bytes` (`net.rs:160`, a `<link>` sheet) and the
`@import` loader (`net.rs:118`). `BaseDocument::make_stylesheet`
(`src/document.rs:612-626`), which `process_style_element` and
`upsert_stylesheet_for_node` use for `<style>`, parses the sheet and returns.
Separately, `Resource::Font` is registered with
`font_ctx.collection.register_fonts(font, None)` (`document.rs:719-725`, "TODO:
Implement FontInfoOveride"), so the rule's `font-family`, `font-weight` and
`font-style` descriptors are ignored: a font whose name table says "Noto Sans"
is found only as "Noto Sans". woff sources are skipped (`net.rs:295-302`).

**Smallest fix** for the first half (4 lines plus `pub(crate)` on
`fetch_font_face`; trivial):

```diff
@@ pub fn make_stylesheet(&self, css: impl AsRef<str>, origin: Origin) -> DocumentStyleSheet
             AllowImportRules::Yes,
         );
+        // <style> elements: fetch their @font-face sources, as net.rs does
+        // for linked and imported sheets.
+        crate::net::fetch_font_face(self.id, &data, &self.net_provider, &self.guard.read());
```

The descriptor half (register under the rule's family, weight and style) is a
day: `fetch_font_face` has to carry the descriptors to `Resource::Font`, and
`register_fonts` takes them as its `FontInfoOverride` argument.

**Newer releases.** The first half is fixed in 0.3.0-beta.4
(`src/document.rs:1225`, in `add_stylesheet_for_node`); the descriptor half was
not checked.

**Workaround without touching the printer.** Yes, and it is what a one-call
print path should do anyway: surimono reads the page's `@font-face` rules,
resolves each `src` under its own resource policy and hands the bytes to the
printer (`AssetBundle::add_font_bytes`). No markup changes, so it is exact,
while the rule's family equals the font's own name. An aliased family
additionally needs the page's family names rewritten to the font's, and that
is neutral only when the font's own name matches no other `@font-face` family
in the page and one CSS family does not map to several internal family names
(static instances named "X SemiBold" and the like). See F8 in `fulgur-layout-gaps-0.40.md` for fulgur's own part
(it reads no file above the input's folder).

## B4. Inline-level layout: what KaTeX needs and 0.2.4 lacks

Each line is one reproducer inside `<p>…</p>`; `.b` is
`display:inline-block; width:12px; background:#2f6fb0`.

| Reproducer | Chrome | blitz-dom 0.2.4 | Spec |
|---|---|---|---|
| `x<span class="b" style="height:20px;vertical-align:-8px"></span>` (also `8px`, `middle`, `top`, `bottom`) | box moved | box bottom on the baseline, always | CSS 2.2 §10.8.1 |
| `E = mc<sup>2</sup>`, `<span style="vertical-align:10px">` | text raised | font-size change only, same baseline | CSS 2.2 §10.8.1 |
| `a<span style="margin-right:20px">b</span>c`, `padding`, `border` on an inline | space, border and background around `b` | none | CSS 2.2 §8.3, §8.5 |
| `x<span style="position:relative;top:-8px">y</span>`, the same on an inline-block | moved | not moved (a block is) | CSS 2.2 §9.4.3 |
| `<span style="white-space:nowrap">no wrap in this span</span>` in a narrow block | one line | wraps | CSS Text 3 §3 |
| `<div style="width:min-content">min content width</div>` | one word wide | full width | CSS Sizing 3, `min-content` |
| `<td style="height:60px;vertical-align:bottom">` (and the default, `middle`) | at the bottom (middle) | at the top | CSS 2.2 §17.5.3 |
| a line holding an inline-block with text | line height from the box's own baseline | the box's whole height sits above the baseline, so the line grows by the text's descent | CSS 2.2 §10.8.1 |

KaTeX uses all of them: `.katex-strut` and `.pstrut` (inline-block struts with
a height and a negative `vertical-align`), `margin-right` on inline `.mord`
and `.mspace`, `.vlist-t { display: inline-table }` with `.vlist { display:
table-cell; vertical-align: bottom }`, `.vlist > span { position: relative;
top: … }`, `.katex-base { white-space: nowrap; width: min-content }`.

**Root cause.** `build_inline_layout_recursive`
(`src/layout/construct.rs:845-929`) turns an inline element into a Parley style
span and nothing else: its margin, padding, border and position never reach
layout. An inline-level box becomes a Parley `InlineBox` with a width and a
height only; parley 0.6.0 has no vertical alignment (its `InlineBox` is `id`,
`index`, `width`, `height`) and seats every box with its bottom on the
baseline. `stylo_to_parley::style` (`src/stylo_to_parley.rs:105-207`) passes
no wrap mode (also `font_features: &[]`, default `word_spacing` and `locale`).
stylo_taffy 0.2.0 maps `min-content`, `max-content` and `fit-content` sizes to
`auto` (`src/convert.rs:76-81`).
Tables are laid out as a CSS grid (`src/layout/table.rs`), where a cell is a
grid item placed at the start.

**Fix size.** Deep at 0.2.4: it is the inline formatting context itself.

**Newer releases.** 0.3.0-beta.4 with parley 0.12.0 passes `vertical-align`
for spans and boxes (`src/stylo_to_parley.rs:349-396`,
`src/layout/construct.rs:1218`, `:1292`) and the wrap mode
(`stylo_to_parley.rs:331`). Inline margins and padding, relative inline boxes
and table-cell `vertical-align` were not found there by search; treat them as
open questions for the issue.

**Workaround without touching the printer.** None that keeps the DOM. What
surimono can do is not print KaTeX's HTML at all: render each formula to a
measured SVG island (KaTeX's layout is fully specified by its CSS and font
metrics, which surimono-gfx already measures) and hand fulgur an `<svg>`, as it
does for charts. That is new work in surimono (days), not a rewrite of CSS.

## B5. Tables are a grid: column widths, `<col>`, cell alignment

```html
<table style="width:100%;border-collapse:collapse">
<tr><th>Region with a long header</th><th>Q1</th><th>Q2</th></tr>
<tr><td>Kanto</td><td>1,200</td><td>980</td></tr></table>
<table style="width:100%;table-layout:fixed"><col style="width:50%"><col style="width:20%"><col style="width:30%">
<tr><th>Fixed</th><th>Q1</th><th>Q2</th></tr></table>
```

**Chrome:** auto layout shares the spare width in proportion to the columns'
max-content widths (first column widest; CSS 2.2 §17.5.2.2); fixed layout
takes the `<col>` widths (§17.5.2.1); collapsed borders are drawn once
(§17.6.2).

**blitz-dom 0.2.4:** the spare width is split equally between the `auto`
columns; `<col>` is ignored, so the fixed table is three equal thirds; collapsed
borders are drawn twice; cell content is at the top (B4).

**Root cause.** `build_table_context` (`src/layout/table.rs:40-98`) builds a
taffy grid: one track per column from the first row's cell widths
(`:175-190`), `auto` otherwise, and taffy's grid algorithm stretches `auto`
tracks equally. `DisplayInside::TableColumn` and `TableColumnGroup` are
skipped (`:232-235`); `rowspan` is not read.

**Fix size.** Deep (a table layout algorithm). **Newer releases:**
0.3.0-beta.4's `table.rs` is 851 lines against 335, with `<col>`, `rowspan`
and `max-content` tracks; not verified by running.

**Workaround without touching the printer.** Not yet. Explicit cell widths on
the first row are honoured by 0.2.4, so Chrome's column widths could be
written there. But surimono does not have Chrome's widths: its own layout
queries (phase 3) sit on this same Blitz table code and would return the wrong
ones. It needs a table layout of surimono's own first. Even then it fixes
widths, not borders or vertical alignment.

## B6. The viewport is whole pixels

`Viewport::window_size` is `(u32, u32)` (blitz-traits 0.2.0 `src/shell.rs:51`),
and layout takes the containing block from it
(`src/document.rs:1198-1209`). A print content box is rarely whole pixels:
A4 with 15 mm margins is 680.315 px. fulgur passes `680`; see F6 in
`fulgur-layout-gaps-0.40.md`, where the effect (a line that fits in Chrome
wraps) and a workaround are described. A float size, or a documented way to
give the root a fractional size, would let an embedder print exact pages. Still
`u32` in 0.3.0-beta.4 (`src/document.rs:1595`).
