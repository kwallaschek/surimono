# Raikiri as the layout behind the seam (E3-0R)

A research spike, no code. Question: should phase 3 answer layout queries
(`getBoundingClientRect`, `offsetWidth`, `getComputedStyle`, `getClientRects`)
from Raikiri, the layout fulgur's maintainer is building "to replace fulgur's
layout engine", from Blitz (the `Engine`/`Laid` seam of #179, `DESIGN.md`
"Layout queries (phase 3)" on `e3-00`), or from both.

Read: the published sources of `fulgur-raikiri` 0.41.0, `raikiri-html`,
`raikiri-dom`, `raikiri-style`, `raikiri-traits`, `raikiri-layout-style`,
`raikiri-layout-geometry`, `raikiri-svg`, `raikiri-net` (all 0.0.6) and `shodo`
0.0.38, with the dependency versions of the printer matrix's driver
(`fixtures/tools/raikiri-print/Cargo.lock`). Source references are to those
crates. Measured: a scratch prototype outside the repository, its own Cargo
workspace on `fulgur-raikiri =0.41.0` whose lock is the driver's lock plus its
own root package (same 316 packages), release build, rustc 1.89.0, this machine
(2 cores); Chromium 153.0.8010.12 through Playwright for the browser's numbers.

**In short.** Raikiri gives every box by node before it paints, through public
API for the paged layout and through hidden but public functions for the
unfragmented one. On the probe pages its boxes are Chrome's to within half a
pixel almost everywhere (the 107-node report page: 61 of 61 elements exact; Blitz
puts that page's last row 52 px higher), where Blitz has the gaps `DESIGN.md`
lists. It costs 2 to 5 times Blitz's time and 2 to 5.5 times its memory per
layout, is 0.0.x software that changes weekly, and adds 108 crates. The
recommendation (section 8) is Raikiri behind the seam now, with Blitz kept as it
is and not developed further.

## 1. Input: what Raikiri lays out

**fulgur-raikiri** takes text only: `render(path, &Config)`,
`render_with_options`, `render_html(&str, base_dir, &Config)`,
`render_html_with_options` and `render_streaming` all return PDF bytes
(`lib.rs`). Nothing in the crate returns a layout.

**raikiri-html**, a published crate of its own and the one fulgur-raikiri
calls, has the layout as public API:

- `parse_html_with_resources(impl Read, &RenderResources) -> HtmlDocument`
  (`resources.rs`): html5ever into a `raikiri_dom::Document`, the rule tree
  and the first page's cascade. `parse_xhtml_with_resources` is the XML twin.
- `layout(&HtmlDocument, PageDefaults, LayoutConfig, LayoutOptions) ->
  LayoutStatus::Completed(DocumentLayout)` (`document_layout/mod.rs`). This is
  the call fulgur-raikiri's `with_layout` makes before it paints.
- An `HtmlDocument` is made only by those two parsers: its fields are
  `pub(crate)` and the function that builds one from a parsed tree,
  `document_parse::assemble_document`, is `pub(crate)`. So the documented
  `layout()` takes HTML text, never a tree.

**A tree can be handed over one level down**, through public but partly
`#[doc(hidden)]` API:

- `raikiri_dom::Document` is a public, mutable arena: `Document::new`,
  `append_element(parent, tag, inline_style)`, `append_text`,
  `set_element_attributes`, `set_element_namespace`, `insert_child_before`,
  `detach_from_parent`, `add_stylesheet` (`document.rs`). It is built for a
  script host: it already has state "a script host selected" (`select_image_url`,
  `set_focused_element`), canvas bitmaps (`canvas_fill_rect`) and
  `set_element_animation_style`, and its comments plan a "future mutation
  runtime".
- `UncascadedDocument::new(dom, author_sheets, user_sheets, index, quirks)`,
  `raikiri_html::build_rule_tree` and `raikiri_style::cascade_with_options` are
  public, so the cascade can be run on a tree we built.
- The layout itself is then `raikiri_dom::layout_single_page(&mut Document,
  &CascadeResult, PageBox)` or `layout_pages` (`layout/page_pipeline.rs`), both
  `pub` and `#[doc(hidden)]`.

**Node ids.** A node's id is its index in the arena, in creation order, index 0
the document (`NodeId(u64)` in `raikiri-traits`). Nothing keeps source
positions. So:

- *Built by us* (path B below): we create every node, so a `Vec` from arena
  index to our node id is the whole mapping, and it can't drift.
- *Serialised and parsed again* (path A): the parser makes the ids. A parallel
  walk of both trees breaks where the HTML parser changes a tree it is given
  (implied `html`/`head`/`body` and `tbody`, foster parenting out of tables,
  adjacent text merged, a script-built `<p><div>` split), and a marker
  attribute on every element of the measuring copy costs bytes and is visible
  to attribute selectors. But path A measures exactly what prints, since the
  printer parses our serialised output too, including where a script-built
  tree doesn't survive the round trip; path B measures the live tree, as a
  browser does.

## 2. Output: geometry before painting

**Public, paged.** `DocumentLayout::pages()` gives each `Page`, and
`Page::fragments()` each placement of a node on it (`raikiri_dom::Fragment`,
`page_projection/fragment.rs`): `node() -> NodeId`, `kind()` (`Box`, `Text`,
`Replaced`), `rect()` (border box in CSS px from the page box's top-left
corner), `content_rect()`, `line_range()`, `fragment_index()`,
`is_first_fragment()`, `is_last_fragment()`. Beside them `Page::text_runs()`
(`PositionedGlyphRun`, glyphs with positions), `Page::computed(node)`,
`Page::dom()` (`DomView`) and `DocumentLayout::is_rendered(node)`.
fulgur-raikiri's painter (`paint::paint_document`) reads exactly these, so
**`Laid` can read the geometry that prints, by node, with no upstream change**.
An inline element gets one `Box` fragment per line, so `getClientRects` is a
filter over fragments.

**Hidden, unfragmented.** Scripts in a browser measure one continuous layout,
not pages. After `layout_single_page`: `Document::layout_origin_of(node)` (the
border-box origin "in the unfragmented layout coordinate space",
`layout/ifc/parent.rs`), `Node::layout() -> &BoxLayout` (location, size,
border, padding, margin, scrollbar size, scrollable overflow;
`raikiri-layout-geometry`), `Node::ifc_inline_boxes()` (`InlineBoxPiece` per
line: border box, content box, line index) and `Document::scene_geometry()`
(every node's border box in document coordinates plus its line pieces,
`scene_geometry.rs`). All `#[doc(hidden)]`.

**Which one scripts should see.** CSSOM View has no pages, and Chrome-based
tools run scripts in one continuous layout. The paged coordinates don't continue
exactly from page to page: on the 2,000-row report page cut into 600 px pages,
the last row is 444 px lower than in the unfragmented layout (and in Chrome),
because a row that would cross a page's end moves to the next page. Sizes are
the same in both on every probe page. So `Laid` would take positions from the
unfragmented layout, which is hidden API, and could check sizes against the
paged one in tests.

**If the boxes were only reachable inside painting**, the draft for upstream
would be one public function; since they aren't, it is optional and small, for
`docs/upstream/` if the orchestrator wants it:
`HtmlDocument::from_parts(UncascadedDocument, &RenderResources) ->
Result<HtmlDocument, RenderError>` (the body of `assemble_document`, made
public), so that a tree built through `raikiri_dom::Document` reaches the
documented `layout()`; and `layout_single_page`, `Document::layout_origin_of`,
`Node::ifc_inline_boxes` and `Document::set_ifc_parallel_build` documented
instead of hidden, or one `layout_continuous(&HtmlDocument, PageBox,
&RenderResources) -> ContinuousLayout` with `border_box(NodeId)` and
`client_rects(NodeId)`.

## 3. Style

Raikiri has its own cascade, `raikiri-style` (about 83,000 lines without
tests): rule tree, layers, `@page` and margin-box cascades, custom properties,
counters, on `cssparser` 0.37 and `selectors` 0.40 (Servo's crates, MPL-2.0,
already exceptions in `deny.toml`). No stylo.

- **Media is print.** `MediaContext::default()` is print with a nominal
  5in × 3in page (480 × 288 px) unless `MediaContext::with_viewport` says
  otherwise (`media.rs`), and `layout()` refuses a screen context
  (`document_layout/mod.rs`). fulgur-raikiri passes `LayoutConfig::default()`.
  Blitz, and fulgur's Blitz backend, cascade as `screen`.
- **Computed values** are a typed struct, `ComputedValues`, public through
  `raikiri_html::computed` and `Page::computed`. A CSSOM serialisation exists
  for about 60 properties, `raikiri_style::ComputedProperty::{from_name,
  serialize}` (`computed/cssom.rs`, `#[doc(hidden)]`): fonts, text, colour,
  white space, overflow, writing mode, spacing, `aspect-ratio`,
  `column-span`. None of the box properties Chart.js reads (`width`,
  `height`, `padding-*`, `border-*-width`, `box-sizing`, `display`,
  `position`, `max-*`). So `getComputedStyle` can be answered, with our own
  serialisation of those fields and used values from the boxes, or with the
  list extended upstream.
- **The style attribute** is kept as text on the element (`inline_style`) and
  parsed by the cascade. Our `style_attr.rs` validates declarations with stylo
  because stylo is "the parser the printer uses" (DECISIONS 2026-11-04); under
  Raikiri the printer's parser is `raikiri-style`, which accepts and refuses
  different things.

## 4. Fonts and determinism

- **Only the caller's fonts.** `FontCollectionBuilder::new()` starts with
  system fonts off; `system_fonts(false)` keeps them off and
  `RenderFonts::is_bundled_only()` says so (`font_collection.rs`). The driver
  goes through fulgur-raikiri's `assets::fonts`, which builds exactly that.
  Every generic family (`serif`, `sans-serif`, `monospace`) maps to the whole
  bundle in registration order: there is no per-generic mapping as surimono-gfx
  sets for Parley.
- **Three ways to the installed fonts**, all when a `Document` has no font
  collection: `Document::normal_line_height` (`document.rs`),
  `layout_single_page` on its first run (`page_pipeline.rs`) and standalone
  text (`layout/ifc/standalone.rs`) call `system_font_collection()`. A document
  parsed with `RenderResources::fonts`, or given `set_font_collection` before
  its cascade, never reaches them.
- **`@font-face`** loads only through the resources' network provider; with
  none, nothing is fetched.
- **It links fontconfig.** `raikiri-dom` turns on shodo's `system-fonts`,
  which turns on fontique 0.11.1's `system`, which links
  `yeslogic-fontconfig-sys` without `dlopen`: the prototype and the driver
  binary need `libfontconfig.so.1` at load time (`ldd`), where the surimono CLI
  and fulgur 0.40.0 need no font library. For precompiled gems our crate would
  turn on fontique's `fontconfig-dlopen` feature.
- **Threads.** Paragraphs are shaped on rayon's global pool when a pass has 32
  or more (`layout/ifc/assign.rs`, `DEFAULT_PARALLEL_THRESHOLD`), "the same as
  when built in sequence". `Document::set_ifc_parallel_build(false)` (hidden)
  turns it off per document; the public `layout()` gives no way to, short of
  running it inside a one-thread rayon pool.
- **Stack.** Deep layouts grow the stack on the heap (`stacker::maybe_grow`,
  `layout/dispatch.rs`), which `MAX_LAYOUT_DEPTH` bounds.
- **Limits** of its own: `RenderLimits` (32 MiB of input, 1,000,000 nodes,
  rule-tree and cascade limits), a fragment budget, shodo's paragraph limits,
  and an abort signal (`LayoutConfig::signal`) that is looked at between the
  stages of pagination, never inside one layout pass.
- **Measured** (section 6, "Determinism"; section 5 for memory over 10,000
  layouts): the same numbers twice, across processes and with one thread.
- **Output**: layout warnings go to standard error (`eprintln!` in
  `raikiri-dom`'s `diag.rs`, no observer can be passed to `layout_single_page`
  yet); nothing is printed on standard output, where the CLI writes the page.

## 5. Cost

The e3-00 prototype's report page (`tests/layout_real.rs` `report`: flex rows of a
label and a figure, a style attribute a row), the same HTML with an `id` on each
row; Raikiri counts 105, 10,005 and 47,505 nodes where surimono-dom counts 107,
10,007 and 47,507 (the document node and the doctype). Release builds, fixture
fonts, an 800 × 600 page; each row is three rounds after one warm-up layout in the
process, all runs while holding the fleet's build lock so no build ran beside
them. Blitz is the e3-00 prototype's own `layout_cost` (`--ignored`), rerun in a
worktree of `origin/e3-00` on the same machine the same evening, which also
copies the tree and lays it out (its "first layout" column).

| Page | Blitz (e3-00) | Raikiri A: parse and cascade, then lay out | Raikiri B: build, cascade, lay out | Raikiri B, one thread | Raikiri paged `layout()` (what prints) |
|---|---|---|---|---|---|
| 107 nodes | 1.1 to 1.4 ms | 0.5 to 0.7 + 2.4 to 2.7 ms | 0.05 to 0.14 + 0.4 + 2.2 to 2.6 ms | 0.05 + 0.4 + 1.7 to 1.9 ms | 3.3 to 3.5 ms, 1 page |
| 10,007 nodes | 58 to 68 ms | 34 to 56 + 248 to 281 ms | 6 to 9 + 22 to 28 + 255 to 282 ms | 7 to 14 + 21 to 28 + 233 to 242 ms | 393 to 516 ms, 75 pages |
| 47,507 nodes | 336 to 423 ms | 266 to 299 + 1,228 to 1,282 ms | 50 to 84 + 124 to 144 + 1,287 to 1,375 ms | 36 to 42 + 93 to 109 + 1,059 to 1,139 ms | 2,326 to 2,470 ms, 352 pages |

- **Per layout Raikiri takes 2 to 3 times Blitz's time on the small page and 4 to
  5 times on the large ones**, about 25 µs a node against Blitz's 6 µs. Almost
  all of it is the layout pass; the cascade is 22 to 28 ms at 10,007 nodes.
- **The round trip through HTML is cheap**: at 10,007 nodes, reading our tree
  into Raikiri's arena (B's build) is 6 to 14 ms, parsing the page's HTML about
  10 to 30 ms (A's parse and cascade less B's cascade), and serialising it
  (Raikiri's own `serialize_inner_html` as a stand-in for our `to_html`) 1.5 to
  3.2 ms. Either way it is about a tenth of the layout or less.
- **One thread is no slower here.** With rayon off (`set_ifc_parallel_build(false)`,
  `RAYON_NUM_THREADS=1`) the layouts are as fast or faster on this 2-core
  machine.
- **Reading boxes back** is 1.1 to 1.9 ms for 2,000 rows' border boxes and 6 to
  7 ms for 9,500 (`layout_origin_of` walks up the tree for each).
- **Memory.** Measured as e3-00 does (resident growth over three layouts, the last
  one alive): Raikiri path A 1.9 MiB, 175 MiB and 813 MiB against Blitz's 1.0,
  55 and 147 MiB, so 2 to 5.5 times. One layout in a fresh process: 1.2 MiB,
  101 MiB (peak 121 MiB) and 401 to 480 MiB (peak 506 MiB); the paged
  `layout()` 1.9, 154 and 592 MiB. About 10 KiB a node.
- **No growth over 10,000 layouts.** The 107-node page laid out 10,000 times in
  one process, paths A and B in turn, each dropped: resident 20,256 KiB from the
  100th layout to the 10,000th, one thread, 2.58 ms a layout. With rayon on: two
  more threads, 3.37 ms a layout, resident 21,092 to 21,716 KiB over 2,000.
- **Real-world pages** (surimono's output of `marked-report` and `d3-charts`, 94
  and 198 nodes): parse and cascade 0.6 and 0.9 ms, layout 1.7 to 2.1 and 0.4 to
  0.5 ms, paged `layout()` 2.6 to 3.0 and 1.4 to 1.7 ms.
- **The page shapes a review found slow with blitz-dom 0.2.4**, long documents
  and long table cells: compared with the printer matrix's driver and stock
  fulgur 0.40.0 printing them, Raikiri was the faster engine on both. The pages
  and numbers are with the orchestrator.
- **Bounds.** At 25 µs and 10 KiB a node, a layout of a page at `max_nodes`
  (100,000) would take about 2.5 s and 1 GiB. `max_layout_nodes` (proposed in
  `DESIGN.md` "Limits") would have to be about 20,000 with Raikiri to keep one
  layout near half a second and 200 MiB.

## 6. Agreement with Chrome

Chromium 153.0.8010.12, `getBoundingClientRect` of every element under `<body>`
in an 800 × 600 window, fixture fonts through `@font-face`; Raikiri on the same
HTML on an 800 × 600 page without margin, fixture fonts only, read from the paged
`layout()`'s fragments (the union of a node's fragments). Probe pages:

| Page | Elements | Within 0.5 px of Chrome | Worst |
|---|---|---|---|
| report, 20 rows (107 nodes; rows, labels and figures) | 61 | 61, all exact | 0 |
| boxes: e3-00's `BOXES`, `vw`/`vh`, a positioned box, a table, canvases with and without `width`/`height`, an inline `<svg>`, floats, flex, grid | 27 | 12; 26 of 26 without the block canvas | a `display: block` canvas with `width=50 height=20` is 800 × 320 (Chrome 50 × 20), which moves everything under it; without it, 0.016 px |
| text: an inline-block of "Hello, world" and wrapped paragraphs in 12px Noto Sans, a span in running text, `line-height: normal`, bold, Japanese with and without `text-autospace`, an inline-block in a line, a shrink-wrapped label | 28 | 27 | the span in the Japanese paragraph with the initial `text-autospace`, 289.67 wide against 296.72: R1 of `fixtures/printers/NOTES.md`; with `no-autospace` 0.05 px |
| gaps: the root's margin, an absolute box in a paragraph, a block `<svg>`, `<img width height>`, shrink-to-fit, a CSS table, a list, two columns, `pre`, `box-sizing`, `min-width` over `max-width` | 17 | 17 | 0.5 px |

Where e3-00's tests and `DESIGN.md` ("What blitz-dom 0.2.4 can't answer") give
Blitz's numbers for the same cases:

| Case | Chrome 153 | Raikiri | Blitz (e3-00) |
|---|---|---|---|
| Report page, bottom of the 20th row | 440 | 440 | 388 (`layout_cost`: rows 19.4 high, `line-height: normal` as 1.2 × 12px) |
| Same, 2,000 rows | 44,000 | 44,000 (unfragmented) | 38,800 |
| Inline-block "Hello, world", 12px Noto Sans | 67.578 wide | 67.578 | 68 (rounded up) |
| `<span>` in running text | 9.86, 141, 27.28 × 17 | 9.86, 141.5, 27.28 × 17 | 0, 0, 0 × 0 |
| `line-height: normal`, 16px Noto Sans, two lines | 44 high | 44 | 38.4 by its rule |
| `html { margin: 7px }`, the first box | at 7, 7 | 7, 7 | 0, 0 |
| Block canvas `width=50 height=20` | 50 × 20 | 800 × 320 | 800 × 0 |
| Inline canvas `width=50 height=20`; no attributes | 50 × 20; 300 × 150 | the same | no box |
| Inline `<svg width=120 height=40>` | 120 × 40 | 120 × 40 | 0 × 0 |
| Two `float: left` blocks | side by side | side by side | stacked |
| Absolute box in a paragraph | 50, 200 | 50, 200 | a stand-in position |

The text page's inline boxes sit up to 0.5 px lower than Chrome's (Chrome snaps
baselines to whole pixels); flex items differ by 0.005 px (Chrome's 1/64 px
layout unit). The unfragmented layout (`layout_single_page`) gives the paged
numbers on every page here except `vw`/`vh`: the cascade a parse makes uses the
print media's nominal 480 × 288 page for them, so the unfragmented path has to
run its cascade again with `CascadeOptions::viewport` set to the page area, as
the paged pipeline does (the prototype's path B sets it).

**Determinism.** Every fragment and glyph run of six pages (the four probes and
the two real-world outputs) is the same twice in one process (once a font's
handle, `FontId`'s blob number, a process-wide counter, is left out), the same in
a second process and the same with one rayon thread. The driver prints
`marked-report` to byte-identical PDFs twice and with one thread.

## 7. Stability, licences, build

- **Versions** (crates.io, 13 Nov 2026): `fulgur-raikiri` 0.41.0 and the
  `raikiri-*` crates 0.0.6 were all published on 1 Nov 2026, after a 0.0.0
  placeholder (25 Oct for `raikiri-*`); `fulgur-raikiri` has 29 downloads.
  `shodo` had 43 releases from 0.0.0 (26 Sep) to 0.0.42 (9 Nov), four since
  0.0.38 (24 Oct), which `raikiri-dom` pins exactly (`=0.0.38`, and
  `taffy =0.15.0`). The READMEs: "The 0.0.x releases make no promise of API
  stability: any release may change the public API" (raikiri), "Early
  development: public APIs may change" (shodo).
- **Repositories**: github.com/fulgur-rs/raikiri and github.com/fulgur-rs/shodo
  weren't readable from here, so no commit history was looked at.
- **Licences**: every crate under `fulgur-raikiri` is under a licence
  `deny.toml` allows; MPL-2.0 only for `cssparser` 0.37, `cssparser-macros`,
  `selectors` 0.40 and `dtoa-short`, which are exceptions there already.
  Nothing new to allow.
- **MSRV** 1.89.0 for every `raikiri-*` crate, shodo and fulgur-raikiri: our
  toolchain.
- **Build**: `raikiri-html` with its dependencies is 184 crates on Linux; 76
  are in our lock already, 41 are a second version of one we have (ICU 2.4.0
  next to our 2.2.0, `harfrust` 0.12.0 and 0.14.0 next to 0.3.2, `skrifa`
  0.48.0, `fontique` 0.11.1, `html5ever` 0.40.1, `cssparser` 0.37.0,
  `selectors` 0.40.0, `taffy` 0.15.0), 67 are new (resvg, usvg, tiny-skia,
  image decoders, rayon, stacker, fontconfig bindings). About 300,000 lines of
  Raikiri and shodo without tests, against 10,500 in blitz-dom 0.2.4. A clean
  release build of the prototype's 316 crates took 11 min 15 s here (16 min of
  CPU, 2 jobs).

## 8. Recommendation

**Raikiri behind the seam now; Blitz kept as it is and not developed further.**
E3-01 builds the queries against a Raikiri `Engine`; the Blitz engine of #179
stays, frozen at the prototype's scope (border boxes of block-level boxes), as
the engine for callers who print with stock fulgur until fulgur's default
changes, and gets none of the Blitz-specific work `DESIGN.md` plans (the canvas
and svg size sheet, inline rectangles from Parley).

Why, in numbers:

- **It measures what Chrome measures.** 61 of 61 elements of the report page
  exact, 26 of 26 boxes, 27 of 28 text boxes and 17 of 17 gap probes within
  0.5 px. Blitz is 52 px short on the 20-row page and 5,200 px on the 2,000-row
  one (`line-height: normal`), answers zeros for inline elements, ignores the
  root's margin, has no floats and sizes no inline canvas or svg; each of those
  would be E3-01 work on Blitz, and Raikiri already does them.
- **It prints what Chrome prints**, which is the directive: 5 of 6 real-world
  pages and 8 of 13 gap reproducers against fulgur's 2 and 2
  (`fixtures/printers/NOTES.md`). The fixes we would make to measuring on Blitz
  would make measuring disagree with Blitz's own printing (it prints with
  `line-height` 1.2 too), so they buy nothing unless the printer is Raikiri.
- **It is where fulgur is going**, so work on Blitz is the work that is thrown
  away (DECISIONS 2026-11-12, Q-ORC-01).
- **Its cost is affordable where layout runs.** About 3 ms for a 100-node page
  and 0.3 s for 10,000 nodes per layout, 2 to 5 times Blitz, against a
  Chrome-based tool's whole render; the layout runs only when a script asks, the
  mirror is dropped when scripts settle, and 10,000 layouts leave memory flat.
  The memory per node (about 10 KiB) is the number to bound: `max_layout_nodes`
  near 20,000.
- **What argues against it, and how to live with it**: 0.0.x crates whose API
  may change at any release (43 shodo releases from 26 Sep to 9 Nov); the
  unfragmented geometry is hidden API; 108 crates more to build (184 under
  `raikiri-html`, about 300,000 lines) and roughly 15 to 20 MB more native code
  in the gem (the layout-only prototype is 21.8 MB stripped, 8.8 MB gzipped,
  against the surimono CLI's 14.6 MB and 6.1 MB; little of it is shared);
  Japanese text spaced against Latin letters and digits unlike Chromium (R1). Exact pins (`=0.0.6`, and fulgur-raikiri's `=0.41.0` for the
  printer), the seam (one file knows the engine), the printer matrix at every
  fulgur release, and a rule `:root { text-autospace: no-autospace }` in the
  mirror's UA sheet and, if Q-PKG-02 takes it, in printer-safe output contain
  each (as an author rule on the text probe it brings all 28 boxes within
  0.5 px of Chrome; on the paragraph it closes R1's reproducer, `NOTES.md`).

"Blitz now and Raikiri later" means writing the Blitz gap fixes first and then
the Raikiri engine anyway. "Both, developed alike" doubles every query's tests
and COMPAT rows for an engine that is leaving. Neither is cheaper.

**What the seam in #179 needs for Raikiri:**

1. **`LayoutSetup::fonts`**: font bytes with their family names, not a Parley
   `FontContext`. The Raikiri engine builds one `RenderFonts`
   (`FontCollectionBuilder::new().system_fonts(false)`, the bytes in the
   printer's order) per setup and clones it per mirror, and calls
   `set_font_collection` before every cascade, so no path reaches
   `system_font_collection()`. Raikiri maps every generic family to the whole
   bundle in order, so the font that should answer `sans-serif` goes first, and
   the mirror gets fulgur-raikiri's user sheet `@layer { :where(html) {
   font-family: sans-serif } }`.
2. **Page geometry**: the setup carries the printer's page size and default
   margins, not a content width. Raikiri takes the page area from the
   document's own cascade (an author `@page` beats the caller's margins, given
   as a user `@page` sheet as fulgur-raikiri's `page_stylesheet` does), so the
   engine works the width out per document. `PageBox` plus that sheet is the
   input; vw/vh need `CascadeOptions::viewport` set to the page area.
3. **Media**: Raikiri cascades as `print` (the parse's cascade and `layout()`
   both). The mirror should too, to measure what prints, which leaves
   `matchMedia('print')` false for scripts as in Chrome-based tools while the
   boxes come from print styles: a decision for DECISIONS.
4. **The tree**: the mirror is built through `raikiri_dom::Document`'s mutators
   (path B), with a `Vec` from arena index to our node id; `<style>` text in
   document order becomes `UncascadedDocument`'s author sheets, the style
   attribute the element's `inline_style`. A test that also serialises and
   parses (path A) and compares the boxes catches the trees HTML can't carry.
5. **`Laid`**: `border_box` from `layout_origin_of` and `Node::layout().size`;
   `client_rects` from `ifc_inline_boxes` (per line) or the paged fragments;
   `box_edges` from `BoxLayout`'s border, padding and scrollbar sizes;
   `computed_value` from `ComputedValues`, through `ComputedProperty::serialize`
   where it covers the property and our own serialisation of the box properties
   elsewhere.
6. **Ownership and threads**: `layout_single_page` takes `&mut Document`, so the
   mirror stays one document per layout, owned by the engine. The types are
   `Send` (`Document`, `HtmlDocument`, `DocumentLayout`, `RenderFonts`), and
   nothing is kept per thread as stylo does, but a work counter
   (`layout/ifc/inline_boxes/work.rs`); process-wide there are the prepared UA
   rule tree, id counters and an image-cache budget. `set_ifc_parallel_build(false)` on
   every mirror keeps rayon's pool out of the sandbox at no cost (section 5).
7. **Style-attribute validation** (`style_attr.rs`) asks stylo because stylo was
   the printer's parser; with Raikiri it should ask `raikiri-style`, or keep
   stylo for the caps only.
8. **The build**: `raikiri-html` and `raikiri-dom` as dependencies of crates/dom
   with exact pins (root `Cargo.toml`, orchestrator), and fontique's
   `fontconfig-dlopen` feature turned on, so that the gem has no load-time
   dependency on `libfontconfig` (to be checked on a built gem).
