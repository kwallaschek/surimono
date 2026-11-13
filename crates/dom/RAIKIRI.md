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

**Status: draft, being filled in.**

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

## 5. Cost

To come.

## 6. Agreement with Chrome

To come.

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
  Raikiri and shodo without tests, against 10,500 in blitz-dom 0.2.4.

## 8. Recommendation

To come.
