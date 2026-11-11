# fulgur 0.40.0: print gaps found by surimono's fixtures, with root causes

**Project:** fulgur 0.40.0 (blitz-dom 0.2.4, stylo 0.8.0, taffy 0.9.2, parley 0.6.0, krilla 0.7.0) · **Status:** drafts for Kay to file (one issue each) · **Local patch:** none in surimono's workspace; F1 to F5, F7 and the Blitz and parley fixes they are scored with were proved in a scratch build of fulgur-cli 0.40.0 ("Scores")

Each item was found by printing the same page with Chromium 153 and fulgur 0.40.0
and comparing rasters (surimono `fixtures/tools/compare.mjs`). Q-GFX-01
(11 Nov 2026) traced each one to its source. Gaps whose cause is in Blitz or
parley are in `blitz-print-gaps-0.2.4.md` (B1 to B6) and
`parley-print-gaps-0.6.md` (P1, P2); this file has the ones in fulgur's own
code (F1 to F9) and the scores for all of them.

For every item: a few-line reproducer, what Chrome prints and the spec section,
the cause (file and function in the fulgur 0.40.0 crate), the smallest patch,
its size and risk (trivial / a day / deep), the state of fulgur 0.41.0
(1 Nov 2026, the newest release), and whether surimono could avoid it without
touching the printer. "Checked" means the published crate source was read;
"ran" means the binary was run.

**fulgur 0.41.0 in one paragraph.** The `fulgur` crate became a facade over
`fulgur-blitz` (the 0.40 code, still on blitz-dom 0.2.4, parley 0.6.0) and
`fulgur-core`. Every cause below is unchanged in `fulgur-blitz` 0.41.0
(checked, file by file). 0.41.0 also publishes `fulgur-raikiri`, "Raikiri
layout backend under development for fulgur": a second backend on fulgur's own
layout crates (`raikiri-html`, `raikiri-dom` 0.0.6, "taffy block layout and
shodo inline layout") that does not use Blitz. It is not what `fulgur` or
`fulgur-cli` 0.41.0 run. Its paint code already has per-side border colours
(`src/paint/border.rs`) and a synthetic oblique (`src/paint/text_clip.rs`), so
F1 and F3 may be answered with "fixed in Raikiri". Worth asking in the first
issue which backend fixes should target.

Reproducers assume `fulgur render --font NotoSans-Regular.ttf --font
NotoSans-Bold.ttf -o out.pdf in.html` and `body { font: 14px/22px "Noto Sans" }`.

## F1. One border colour for all four sides

```html
<div style="border-bottom: 4px solid #d33">bottom only, red</div>
<div style="border-style: solid; border-width: 4px; border-color: #d33 #3a3 #33d #cc0">four colours</div>
```

**Chrome:** a red line under the first box; red, green, blue and yellow sides
on the second. CSS Backgrounds 3 §3.1: each side has its own `border-*-color`,
initially `currentcolor`.

**fulgur 0.40.0:** the first line is black (the text colour), the second box is
red on all sides. `border` on all sides keeps its colour only because all four
are then equal. Page `noscript-message` fails for this (100% hue).

**Root cause.** `convert/style/border.rs:11-13`:

```rust
// Border color (use top border color for all sides for simplicity)
let bc = ctx.styles.clone_border_top_color();
style.border_color = absolute_to_rgba(bc.resolve_to_absolute(ctx.current_color));
```

`BlockStyle::border_color` is one `[u8; 4]` (`draw_primitives.rs:732`) and
`draw_block_border` (`draw_primitives.rs:1545-1690`) strokes every side with
it. With `border-bottom` alone the top colour is the initial `currentcolor`.

**Smallest fix** (about 40 lines in two files; trivial, low risk: `None` keeps
today's behaviour for every existing constructor of `BlockStyle`):

```diff
@@ draw_primitives.rs, struct BlockStyle
     pub border_color: [u8; 4],
+    /// Per-side border colors (top, right, bottom, left); `None`: `border_color` on all.
+    pub border_side_colors: Option<[[u8; 4]; 4]>,
@@ convert/style/border.rs, apply_to
     style.border_color = absolute_to_rgba(bc.resolve_to_absolute(ctx.current_color));
+    let side = |c: style::values::computed::Color| {
+        absolute_to_rgba(c.resolve_to_absolute(ctx.current_color))
+    };
+    style.border_side_colors = Some([
+        style.border_color,
+        side(ctx.styles.clone_border_right_color()),
+        side(ctx.styles.clone_border_bottom_color()),
+        side(ctx.styles.clone_border_left_color()),
+    ]);
@@ draw_primitives.rs, draw_block_border
-    let bc = &style.border_color;
+    let [ct, cr, cb, cl] = style.border_side_colors.unwrap_or([style.border_color; 4]);
+    // A side that paints nothing does not make the colours differ.
+    let painted: Vec<[u8; 4]> = [(bt, st, ct), (br, sr, cr), (bb, sb, cb), (bl, sl, cl)]
+        .iter()
+        .filter(|(w, s, _)| *w > 0.0 && *s != BorderStyleValue::None)
+        .map(|(_, _, c)| *c)
+        .collect();
+    let uniform_color = painted.windows(2).all(|p| p[0] == p[1]);
+    let bc = &painted.first().copied().unwrap_or(ct);
 …
     } else if !style.has_radius()
         && uniform_width
         && uniform_style
+        && uniform_color
 …   (per-side branch: each `draw_border_line` call takes its own side's colour
      and `alpha_to_opacity` of it instead of `bc` and `opacity`)
```

Left for a follow-up: four different colours meet at butt ends, not at the
mitred diagonal Chrome draws (CSS Backgrounds 3 §4.4), and a rounded border
with different colours still uses the first painted side's.

**fulgur 0.41.0:** unchanged in `fulgur-blitz` (`convert/style/border.rs:11-13`
is identical; ran, same raster). `fulgur-raikiri` resolves each side's colour.

**Workaround without touching the printer.** Yes for the common case, exact:
when only one side paints (the other three have zero width or `none`), write
that side's colour to all four (`border-color: #d33`). No workaround for boxes
with two or more different colours short of nested boxes.

## F2. Bullet markers touch the text

```html
<ul><li>disc<ul><li>circle<ul><li>square</li></ul></li></ul></li></ul>
<ol><li>decimal</li></ol>
```

**Chrome:** at 14 px Noto Sans the disc is 5 px wide and its right edge is
12 px left of the item's content edge; circle and square likewise. "1." ends
one space before the content edge. CSS Lists 3 §3.5: "CSS does not specify the
precise location of the marker box", so Chrome is the reference: Blink places a symbol
marker with its right edge `⌊2A/3⌋ + 7 − ⌊(⌊2A/3⌋ + 1) / 2⌋` px left of the
content edge, where `A` is the font's ascent rounded to a pixel
(`ListMarker::InlineMarginsForOutside` and `RelativeSymbolMarkerRect`).

**fulgur 0.40.0:** the bullet's ink ends 1.6 px before the text; numbers are
right. The bullet is also not the one Blitz means: with Noto Sans JP it is the
font's own small `•`, 3 px wide. Page `marked-report`.

**Root cause.** Two things.

- The gap. blitz-dom gives symbol markers no trailing space
  (`layout/list.rs:128-132`: `Marker::Char('•')`, while decimal is `"1. "`) and
  leaves the gap to the painter: blitz-paint's `draw_marker` adds 8 px for
  `Marker::Char` (`x_padding`). fulgur's painter does not: `render.rs:3270-3272`
  draws the marker at `x − width`, the glyph's advance.
- The glyph. blitz-dom shapes symbol markers in `"Bullet, monospace,
  sans-serif"` (`layout/list.rs:138-149`) and registers its bundled "Bullet"
  font only in the font context it creates itself
  (`document.rs:205-221`). fulgur passes its own context whenever fonts are
  bundled (`blitz_adapter.rs`, `parse_inner`), so "Bullet" is missing and the
  marker falls back to whichever registered font has U+2022, U+25E6 or U+25AA.
  Passing blitz-dom's `assets/moz-bullet-font.otf` with `--font` brings Blitz's
  bullet back (`pdffonts` then lists `Mozilla_Bullet`).

**Smallest fix** for the gap (about 35 lines in `convert/list_item.rs`; trivial,
low risk): widen the marker's reserved width for symbol markers by Chrome's
distance.

```diff
@@ convert/list_item.rs, try_convert
-        let (marker_lines, marker_width, marker_line_height) =
+        let (marker_lines, mut marker_width, marker_line_height) =
             list_marker::extract_marker_lines(doc, node, ctx);
+        if elem_data
+            .list_item_data
+            .as_ref()
+            .is_some_and(|d| matches!(d.marker, crate::blitz_adapter::Marker::Char(_)))
+        {
+            marker_width = marker_width + symbol_marker_gap(doc, node);
+        }
 …
+/// Chrome's distance from a symbol marker's right edge to the content edge,
+/// less the 0.113 em the Bullet font's glyph already leaves.
+fn symbol_marker_gap(doc: &BaseDocument, node: &Node) -> crate::units::Pt {
+    let font_size_px = /* node's used font-size */;
+    let ascent = /* first line's ascent of the item, or of its first anonymous block; else font_size_px */.round();
+    let offset = (ascent * 2.0 / 3.0).floor();
+    let diameter = ((offset + 1.0) / 2.0).floor();
+    (offset + 7.0 - diameter - 0.113 * font_size_px).max(0.0).as_px().in_pt()
+}
```

The simpler variant, blitz-paint's fixed 8 px, is 3 lines and leaves the bullet
about 2.5 px closer to the text than Chrome at 14 px.

**The whole fix** (a day): draw disc, circle and square as shapes with Blink's
geometry (a filled ellipse, a stroked ellipse and a filled square of
`⌊(⌊2A/3⌋ + 1) / 2⌋` px, at the distance above), as Chrome does, instead of
glyphs. Then neither the Bullet font nor a fallback font decides how a bullet
looks. With the gap alone `marked-report` still fails on the bullets' size
("Scores").

**fulgur 0.41.0:** unchanged (`fulgur-blitz` `render.rs:3305`); ran, same
raster.

**Workaround without touching the printer.** Only by replacing the marker:
`list-style: none` plus an absolutely positioned `li::before`. That changes the
page's CSS and what a tagged PDF calls a list label; not recommended.

## F3. No synthesized italic or bold

```html
<p style="font-family: 'Noto Sans'">upright <em>emphasis</em></p>   <!-- only the regular face is registered -->
```

**Chrome:** slants the upright face. CSS Fonts 4 §2.4 and §2.8
(`font-synthesis-style`, initially `auto`): when the family has no italic or
oblique face the user agent synthesizes one; Blink shears by 1/4 (14°).

**fulgur 0.40.0:** upright. Page `marked-report` (`<em>` in Noto Sans JP, which
has no italic).

**Root cause.** fontique already answers the question: for a run whose family
lacks the style it returns `Synthesis { skew: 14, .. }` (fontique 0.6.0
`src/font.rs:105-119`) and parley exposes it as `Run::synthesis()`.
`extract_paragraph` (`convert/inline_root.rs:474-545`) never reads it, and
`ShapedGlyphRun` (`paragraph.rs:175`) has nowhere to keep it; `draw_shaped_lines`
(`paragraph.rs:807`) draws every run upright. The same holds for
`Synthesis::embolden()` (a bold request on a family with no bold face) and for
the variation settings in `Synthesis::variation_settings()`.

**Smallest fix** (a day with tests: one field, one transform, and the field
added at the 17 places that build a `ShapedGlyphRun`; low risk):

```diff
@@ paragraph.rs, struct ShapedGlyphRun
+    /// Skew in degrees for a synthesized italic or oblique.
+    pub synthetic_skew_deg: Option<f32>,
@@ convert/inline_root.rs, extract_paragraph
                             link,
+                            synthetic_skew_deg: run.synthesis().skew(),
@@ paragraph.rs, draw_shaped_lines
+                    // Shear about the run's baseline start.
+                    let skew = run.synthetic_skew_deg.map(|deg| deg.to_radians().tan());
+                    if let Some(k) = skew {
+                        canvas.surface.push_transform(&krilla::geom::Transform::from_row(
+                            1.0, 0.0, -k, 1.0, k * start.y, 0.0,
+                        ));
+                    }
                     canvas.surface.draw_glyphs(start, &krilla_glyphs, font, &run.text, run.font_size.to_f32(), false);
+                    if skew.is_some() {
+                        canvas.surface.pop();
+                    }
```

Synthetic bold needs a stroke of the fill colour around the glyphs (krilla
draws fill and stroke when both are set); not sketched.

**fulgur 0.41.0:** unchanged in `fulgur-blitz` (no `synthesis` anywhere in
`convert/`; ran, same raster). `fulgur-raikiri` applies a synthetic oblique.

**Workaround without touching the printer.** None in CSS (fulgur has no
`transform` on inline text). The caller can register a real italic face; that
is the honest advice for the guide.

## F4. Inline boxes have no background

```html
<p>before <code style="background:#fd0">inline code</code> after</p>
```

**Chrome:** a yellow box behind "inline code", as high as the font's content
area (CSS 2.2 §10.6.1) and as wide as the text (plus padding and border,
§8.5).

**fulgur 0.40.0:** no background. Page `marked-report` (`code { background }`),
and every syntax theme that colours spans.

**Root cause.** A glyph run takes three things from its element (the node id
Blitz stores in the Parley brush): colour, text decoration and link
(`convert/inline_root.rs:482-485`). `ShapedGlyphRun` has no background and
`draw_shaped_lines` paints none. Backgrounds exist only for boxes with a taffy
layout (`BlockStyle`), which a non-replaced inline does not have.

**Smallest fix** (a day with tests; about 60 lines; low risk): collect the
backgrounds of the inline ancestors between the run's element and the inline
root, and fill a rectangle per run over the run's ascent and descent before
its glyphs.

```diff
@@ paragraph.rs, struct ShapedGlyphRun
+    /// Background colours of the inline boxes this run lies in, outermost first.
+    pub inline_backgrounds: Vec<[u8; 4]>,
+    /// The run font's ascent and descent: the inline box's content area.
+    pub ascent: crate::units::Pt,
+    pub descent: crate::units::Pt,
@@ convert/inline_root.rs, extract_paragraph
+                            inline_backgrounds: inline_backgrounds(doc, brush.id, node.id),
+                            ascent: run.metrics().ascent.as_px().in_pt(),
+                            descent: run.metrics().descent.as_px().in_pt(),
 …
+fn inline_backgrounds(doc: &BaseDocument, span_id: usize, root_id: usize) -> Vec<[u8; 4]> {
+    // walk `parent` from span_id up to (not including) root_id, keeping every
+    // background-color with alpha > 0; reverse so the outermost paints first
+}
@@ paragraph.rs, draw_shaped_lines, before draw_glyphs
+                    for bg in &run.inline_backgrounds {
+                        // fill (start.x, start.y - ascent, run width, ascent + descent) with bg
+                    }
```

Padding, border and margin of an inline box are a separate, deeper gap: Blitz
0.2.4 does not lay them out (B4 in `blitz-print-gaps-0.2.4.md`), so the
rectangle is the text's width exactly.

**fulgur 0.41.0:** unchanged in `fulgur-blitz` (`ShapedGlyphRun` has the same
fields; ran, same raster). The release notes list inline backgrounds for Raikiri (#841).

**Workaround without touching the printer.** Partly: `display: inline-block`
on the element gives it a box fulgur paints, but an inline-block cannot break
across lines, its background is the line's height, not the content area's, and
F5 then applies. Acceptable for short `code` spans, wrong for long ones.

## F5. An inline-block in a wrapped paragraph directly under `<body>` is not painted

```html
<body>
<p>One line: <span style="display:inline-block;background:#fd0">inline-block</span> end.</p>
<p>Two lines: <span style="display:inline-block;background:#fd0">inline-block</span> and then enough
words to wrap this paragraph onto a second line of the page.</p>
</body>
```

**Chrome:** both boxes are painted (CSS 2.2 §9.2.2: an inline-block is an
atomic inline-level box in the line).

**fulgur 0.40.0:** the first box is painted; the second leaves a hole of its
size: no background, no text. The same paragraph inside a `<div>` prints.
Every inline-block, inline-flex and inline-table is affected. This is why
`E = mc²` is missing from page `katex-autorender`: KaTeX's `.katex-base` is an
inline-block, the inline formula sits in a two-line `<p>` under `<body>`, and
the display formulas, alone in their paragraphs, print.

**Root cause.** The fragmenter has a branch for a body-level inline root with
more than one line (`pagination_layout.rs:737-779`): it calls
`fragment_inline_root`, which records a `Fragment` for the paragraph only, and
`continue`s. The block path a one-line paragraph takes records the whole
subtree (`record_subtree_descendants`, `:1095`), and so does the nested path
(`:2233`). The painter looks the inline box up in the geometry table and skips
it without a word when it is not there (`paragraph.rs:904-911`).

**Smallest fix** (12 lines; trivial, low risk) for a paragraph that stays on
one page:

```diff
@@ pagination_layout.rs:755, after fragment_inline_root(...)
+                // The paragraph's inline boxes need geometry too, or
+                // `draw_shaped_lines` finds none and skips them.
+                if frag_count == 1 {
+                    record_subtree_descendants(
+                        &mut self.geometry,
+                        self.doc,
+                        child_id,
+                        page_index,
+                        cursor_y,
+                        body_x + para_x,
+                        0,
+                    );
+                }
```

A paragraph split across pages additionally needs each box recorded on the page
of its line (a day). Independently, a debug log where `paragraph.rs:904` skips
a box would have made this visible.

**fulgur 0.41.0:** the branch is unchanged in `fulgur-blitz`
(`pagination_layout.rs:743-785`); 0.41.0's "split nested inline-root paragraphs
at line boundaries" (#741) adds the same line splitting to the nested path
(`:2749`). Ran fulgur-cli 0.41.0: the reproducer prints as in 0.40.0 (hole
under `<body>`, painted inside a `<div>`).

**Workaround without touching the printer.** Yes, two: wrap the body's
children in one `<div>` (changes what `body > p` selects), or give a body-level
paragraph that holds an inline box `break-inside: avoid`, which sends it down
the block path (`pagination_layout.rs:732-736`; the paragraph then cannot
split across pages). The second keeps the DOM. Ran both with the crates.io
binary: all four boxes of the reproducer are painted.

## F6. The content width is cut to whole pixels

```html
<style>@page { size: 210mm 297mm; margin: 15mm } body { margin: 0 }</style>
<p>… a line that is between 680.0 and 680.3 px long …</p>
```

**Chrome:** the page area is 180 mm = 680.315 px wide and lines break against
that width (Blink's layout unit is 1/64 px).

**fulgur 0.40.0:** lines break against 680 px: a filled block is exactly
510.000 pt wide in the PDF (`42.519688 … 552.5197`), where 180 mm is
510.236 pt. A line that fits by less than 0.32 px in Chrome wraps. On
`marked-report` the first paragraph ends its first line at "the print" instead
of "the print guide", and everything below moves.

**Root cause.** `blitz_adapter.rs:313` passes `viewport_width as u32` to
`Viewport::new`, and `set_viewport_size_px` (`:395`) does the same;
Blitz's viewport is `(u32, u32)` (B6 in `blitz-print-gaps-0.2.4.md`). The
fraction is dropped, not rounded.

**Smallest fix** inside fulgur while Blitz's viewport is integral (a few
lines; trivial, but the risk is medium: percentages and `vw` still see the
integer): give the root element the exact width with a user-agent rule,
`html { width: 680.315px }`, built from the resolved content width in
`engine.rs` (`resolved_content_width_px`, `:263`). Rounding up instead of
truncating (`.ceil() as u32`) is the one-token alternative; it errs toward
fitting, by up to 1 px.

**fulgur 0.41.0:** unchanged (`fulgur-blitz` `blitz_adapter.rs:421`, `:628`;
ran, the same 510.000 pt).

**Workaround without touching the printer.** Yes, exact where the page states
its size: surimono writes `html { width: <page width − margins> }` from the
page's `@page` rule (or the caller's page size). Ran: with
`html{width:680.315px}` fulgur 0.40.0 breaks `marked-report`'s first paragraph
where Chrome does.

## F7. A ligature's characters are missing from the PDF's text

```html
<p>five differently flaky</p>
```

**Chrome:** `pdftotext` gives "five differently flaky".

**fulgur 0.40.0:** "fve diferently faky" with Noto Sans. The page looks right;
search, copy and a screen reader get the wrong words.

**Root cause.** Parley puts a ligature's glyph on the cluster of its first
character; the clusters of the characters it absorbed are "ligature
continuations" with no glyph (`Cluster::is_ligature_start`,
`is_ligature_continuation`). `extract_paragraph` pairs each glyph with its own
cluster's `text_range()` (`convert/inline_root.rs:499-505`), so the absorbed
characters belong to no glyph and krilla has no text for them. The same
pairing is in `convert/list_marker.rs:221` and `convert/mod.rs:914`.

**Smallest fix** (about 25 lines per site; trivial, low risk): extend a
ligature start's range to the end of its last continuation.

```diff
@@ convert/inline_root.rs, extract_paragraph
+                    let ligature_end: BTreeMap<usize, usize> = {
+                        let mut ends = BTreeMap::new();
+                        let mut open: Option<usize> = None;
+                        for cluster in run.clusters() {
+                            let r = cluster.text_range();
+                            if cluster.is_ligature_continuation() {
+                                if let Some(start) = open {
+                                    ends.insert(start, r.end);
+                                }
+                            } else {
+                                open = cluster.is_ligature_start().then_some(r.start);
+                            }
+                        }
+                        ends
+                    };
                     let mut annotated = run
                         .visual_clusters()
                         .flat_map(|cluster| {
-                            let r = cluster.text_range();
+                            let mut r = cluster.text_range();
+                            if let Some(end) = ligature_end.get(&r.start) {
+                                r.end = r.end.max(*end);
+                            }
                             cluster.glyphs().map(move |g| (r.clone(), g))
```

**fulgur 0.41.0:** the pairing is unchanged in `fulgur-blitz`.

**Workaround without touching the printer.** `font-variant-ligatures: none`
would avoid ligatures, but blitz-dom 0.2.4 passes no font features to parley,
so the page cannot turn them off. None.

## F8. `@font-face` files outside the input's folder are not read

```html
<!-- /site/pages/report.html -->
<link rel="stylesheet" href="../lib/katex/katex.min.css">   <!-- its @font-face: url(fonts/KaTeX_Main-Regular.woff2) -->
```

**Chrome:** loads the stylesheet and its fonts.

**fulgur 0.40.0:** reads neither, without a message. With the stylesheet
inlined in a `<style>` element the fonts are still not read, for Blitz's reason
(B3 in `blitz-print-gaps-0.2.4.md`).

**Root cause.** By design: `FulgurNetProvider::resolve_local_path`
(`net.rs:96-107`) accepts only `file:` URLs whose canonical path is inside the
base directory, and the CLI sets the base to the input file's folder
(`fulgur-cli` `main.rs:464-477`); there is no flag to widen it. The design is
sound (offline, no path traversal); what is missing is a way for the caller to
say which other folders are the document's, and a warning when a resource is
refused.

**Ask** (not a patch): a repeatable `--allow-path DIR` (and
`EngineBuilder::allow_path`), and a `log::warn!` in `fetch` when
`resolve_local_path` refuses. A day.

**fulgur 0.41.0:** unchanged (`fulgur-raikiri`'s README states the same rule:
"files outside it are not read").

**Workaround without touching the printer.** Yes: surimono passes font bytes
itself (B3). This is how `fixtures/tools/print-pages.mjs` prints KaTeX today
(`print_fonts`, `--font`).

## F9. Older items, where they stand

The list this file had before Q-GFX-01, with what is now known:

1. **Single-side border colour:** F1.
2. **`flex-wrap` rows / a right-aligned flex item wraps earlier:** the
   reproducer in `docs/COMPAT.md` (a flex header whose right item has
   `<br>`-separated lines) is parley's max-content width, P1 in
   `parley-print-gaps-0.6.md`. `flex-wrap` itself was not re-tested.
3. **Table column widths:** Blitz lays tables out as a grid, B5.
4. **Fractional line heights:** not reproduced with plain paragraphs: for
   `font-size: 10pt; line-height: 1.4`, ten lines and a 3 mm margin, fulgur's
   baselines equal Chrome's to 0.01 pt (both snap line boxes to whole pixels;
   parley's `quantize` mimics Blink). What remains is F6 and that fulgur reads
   taffy's rounded `final_layout` for every box, so a box edge can sit up to
   half a pixel from Chrome's. **Fallback line boxes** (a line that grows for a
   fallback font's taller metrics) was not traced.
5. **`text-spacing-trim`:** not implemented in the stack, P2.
6. **Bold fallback for Japanese in inline SVG text:** as before, the usvg path
   with system fonts (`blitz-svg-document-fonts.md`).
7. **`@page` size rounding:** Chrome's, not fulgur's.

## Scores

Method: `fixtures/tools/print-pages.mjs`'s "floor" (fulgur prints Chrome's own
DOM, `reference/dom.html`, with the fixture fonts) scored by
`fixtures/tools/compare.mjs` against the committed Chromium 153 references.
Shape / colour / hue are worst-window shares; a page passes at or below 10%
each. "Before" is fulgur-cli 0.40.0 from crates.io (`cargo install --locked`);
fulgur-cli 0.41.0 from crates.io scores the same on all four pages and prints
every reproducer in F1 to F5, B1, B2 and P1 pixel for pixel like 0.40.0 (ran).
"After" is fulgur-cli 0.40.0 built from the crate sources with `[patch]`
copies of fulgur, blitz-dom and parley carrying the sketches above (F1, F2's
gap, F3, F4, F5, F7, B1, B2 with its fragmenter half, B3, P1), in a scratch
directory outside the repository. Each fix sat behind a run-time switch; with
all of them off the build's rasters are identical to the crates.io binary's on
all four pages. 11 Nov 2026, the fleet's Linux machine, poppler 24.02.0.

| Page | Before (0.40.0 and 0.41.0) | After | Which fixes |
|---|---|---|---|
| `d3-charts` | **FAIL** 100.0% / 100.0% / 100.0% | pass 0.0% / 0.0% / 0.0% | B1 alone (all others off: the same) |
| `noscript-message` | **FAIL** 100.0% / 0.0% / 100.0% | pass 9.7% / 0.0% / 0.0% | F1 alone (all others off: the same) |
| `marked-report` | **FAIL** 67.5% / 84.8% / 100.0% | **FAIL** 44.6% / 0.0% / 0.0% with the exact content width (F6, as `html{width:680.315px}`); 53.3% / 100.0% / 100.0% without it | F2, F3, F4, B2, F6 together. What is left is 70 near-mismatched pixels, all in the list bullets (F2, "the whole fix"); with blitz-dom's Bullet font passed by `--font` as well, 23.6% / 0.0% / 0.0%, worst at a hollow circle |
| `katex-autorender` | **FAIL** 85.2% / 0.0% / 0.0% | **FAIL** 85.2% / 0.0% / 0.0%; `E = mc2` is back on the page | F5. The layout of every formula still needs B4 |
| `alpine-invoice`, `highlightjs-code` | pass 0.0% / 0.0% / 0.0% | pass 0.0% / 0.0% / 0.0% | no regression |

`marked-report` with the exact width and one fix switched off: without F2
67.5% / 0.0% / 0.0%; without F4 44.6% / 100.0% / 0.0%; without B2 53.9% /
82.4% / 0.0%; without F3 the scores do not move (one slanted word is inside
the tolerance of every window; the difference is visible).

Other checks with the patched build: F7, `pdftotext` gives "five differently
flaky" (before: "fve diferently faky"); B2, the reproducer's boxes have
Chrome's bounding box to the pixel; B3, `pdffonts` lists the `@font-face` font
of a `<style>` element (before: a system font); P1, the flex header has three
lines in its right item; F5, all four boxes of the reproducer are painted.

The two workarounds that need no printer change were run with the crates.io
binary: `d3-charts` with `style="width:…;height:…"` written on each `<svg>`
passes at 0.0% / 0.0% / 0.0%, and `noscript-message` with
`border-color: #d9e2ec` beside its `border-bottom` passes at 9.7% / 0.0% /
0.0%.

## Order: pages fixed per unit of effort

Pages are surimono's six real-world pages (four fail today) and the
checkpoint-2 template, which avoids P1 and F1 by construction.

| # | Gap | Where | Size | Pages it turns green | Without touching the printer |
|---|---|---|---|---|---|
| 1 | B1 block `<svg>` size | blitz-dom | trivial | `d3-charts` (alone) | yes, exact |
| 2 | F1 border colours | fulgur | trivial | `noscript-message` (alone) | yes for one painted side |
| 3 | F5 inline-block not painted | fulgur | trivial | none alone; brings back content that is silently missing (`E = mc²`) | yes (`break-inside: avoid`) |
| 4 | F6 content width | fulgur | trivial | none alone; needed by `marked-report` | yes, exact |
| 5 | B2 trailing empty line | blitz-dom (and fulgur's fragmenter) | trivial | none alone; needed by `marked-report` | yes (DOM text one character shorter) |
| 6 | P1 max-content width | parley 0.6 | trivial | none of the six; any header or address block with `<br>` in a flex row | costly |
| 7 | F2 bullets | fulgur | trivial for the gap, a day as shapes | `marked-report`, with 4, 5 and 9 | no |
| 8 | F7 ligature text | fulgur | trivial | none (not a pixel) | no |
| 9 | F4 inline backgrounds | fulgur | a day | needed by `marked-report` | partly |
| 10 | F3 synthesized italic | fulgur | a day | none by score | no |
| 11 | B3, F8 `@font-face` | blitz-dom, fulgur | trivial, a day | none (the fixtures pass fonts) | yes, exact |
| 12 | B5 tables | blitz-dom | deep (in 0.3) | none of the six (they avoid tables) | partly |
| 13 | B4 inline layout for KaTeX | blitz-dom, parley | deep (partly in 0.3) | `katex-autorender` | no; an island instead |
| 14 | P2 `text-spacing-trim` | stylo, blitz-dom, parley | deep | none (the fixtures opt out) | no |

First three: B1, F1 and F5. Each is a few lines, the first two turn a page
green on their own, and the third is the only gap here where content
disappears. All three also have a workaround surimono can apply to its own
output today, so the pages do not have to wait for a release. `marked-report`
comes next as a bundle (F6, B2, F2 as shapes, F4): about two and a half days
of printer work for one page, though it is the page that looks like most
people's documents.
