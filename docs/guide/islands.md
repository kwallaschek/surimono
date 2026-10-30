# Islands

An island is a chart (or any other drawing) that a JavaScript library renders
from a JSON spec. You write a marker in your HTML; surimono runs the renderer in
a sandbox and puts the SVG where the marker was. This guide covers the marker
format, sizing, the built-in renderers, errors, your own renderers, and fonts.

For setup, see the [Rails guide](rails.md) or the [Rust guide](rust.md).

## The marker

```html
<script type="application/vnd.surimono+json" id="sales" class="chart"
        data-renderer="echarts" data-width="640" data-height="320">
  {
    "xAxis": { "type": "category", "data": ["Q1", "Q2", "Q3", "Q4"] },
    "yAxis": { "type": "value" },
    "series": [{ "type": "bar", "data": [120, 200, 150, 80] }]
  }
</script>
```

| Attribute | Required | Meaning |
|---|---|---|
| `type` | yes | `application/vnd.surimono+json`. Case doesn't matter, and MIME parameters such as `; charset=utf-8` are allowed. |
| `data-renderer` | yes | `echarts`, `vega-lite`, `vega`, or the name of a bundle you registered |
| `data-width` | no | Width in CSS pixels, a whole number from 1 to 16384. Default 600. |
| `data-height` | no | Height in CSS pixels, a whole number from 1 to 16384. Default 400. |
| `id`, `class` | no | Copied to the output element |

Any other attribute on the marker is dropped.

The element's text is the spec, as JSON. surimono checks that it parses before
any JavaScript runs and reports the line and column if it doesn't.

Rules that follow from HTML itself:

- **The text is not entity-decoded.** A browser hands a script its text
  exactly as written, and so does surimono. Write `"Q&A"`, not `"Q&amp;A"`. This
  is also why ERB's plain `<%= spec.to_json %>` breaks the JSON: it turns `"`
  into `&quot;`.
- **`</script>` ends the element**, even inside a JSON string. Write it as
  `<\/script>`, which JSON reads as the same string. Rails' `json_escape` and
  most JSON encoders that target HTML escape `<` as `<`, which also works.
- **Markers inside comments, `<textarea>`, `<title>` or `<svg>` are not
  islands.** They are text or SVG there, so they are left alone.
- **Put markers where a `<div>` is allowed.** The marker becomes a `<div>`, and a
  `<div>` inside `<p>`, `<span>` or `<a>` makes the HTML parser close or move
  elements around it. A `<figure>`, `<section>`, `<td>` or `<div>` is fine.

Islands are rendered in document order. If the document is too ambiguous to
rewrite safely (rare, and only with malformed markup), the whole render fails
with an `html` error instead of guessing.

## What it becomes

```html
<div class="surimono-island chart" id="sales" data-renderer="echarts"><svg …>…</svg></div>
```

The wrapper always has the class `surimono-island`, followed by the marker's own
classes. Everything else in the document is left byte for byte as it was,
including line endings, attribute quoting and the page's other `<script>`
elements.

An inline `<svg>` sits on the text baseline like an image, which leaves a few
pixels of space under it. Most documents want this in their print CSS:

```css
.surimono-island > svg { display: block; }
```

## Sizing

`data-width` and `data-height` are CSS pixels, 96 to the inch, as in a browser.
Some useful widths:

| Page | Margins | Content width |
|---|---|---|
| A4 portrait (210 mm) | 15 mm each side | 680 px |
| A4 portrait | 20 mm each side | 642 px |
| US Letter portrait (8.5 in) | 0.75 in each side | 672 px |

Give the island the size it will have on the page. Charts lay out their axes and
labels for that size, and scaling the SVG afterwards scales its text too.

- **ECharts** draws at exactly the island's size.
<!-- confirm: how the vega renderer uses ctx.width/height when P1-RT-07 lands -->
- **Vega-Lite and Vega** size the chart from the spec's `width`, `height` and
  `autosize`. Set them to match the island, and use
  `"autosize": { "type": "fit", "contains": "padding" }` so axes and legends
  fit inside that size rather than adding to it.

## Built-in renderers

| `data-renderer` | Library | Spec |
|---|---|---|
| `echarts` | Apache ECharts, server-side SVG renderer | An ECharts option object |
| `vega-lite` | Vega-Lite, compiled to Vega, then Vega's headless SVG renderer | A Vega-Lite spec |
| `vega` | Vega's headless SVG renderer | A Vega spec |

The exact versions are listed in [THIRD_PARTY.md](../../THIRD_PARTY.md). In
Rust each library is a Cargo feature, so you can leave out the ones you don't
use.

What to know about each:

**ECharts**
- The spec is JSON, so it can't contain functions. Use the string templates
  ECharts offers, such as `"axisLabel": { "formatter": "{value} 円" }`, or
  format the values before you put them in the spec.
- Animation is switched off for the option and every series. The SVG shows the
  chart's final state.
- Name a registered font in `textStyle.fontFamily` (for example
  `"'Noto Sans JP'"`). Any other name resolves to your registered fonts as
  described in [Fonts in islands](#fonts-in-islands).

**Vega-Lite and Vega**
- Expressions are strings in Vega's own expression language, so they work as
  they do in a browser.
- Put the data in the spec (`"data": { "values": [...] }`). A spec that loads
  data from a URL fails: the sandbox has no network.
- Set the font with `"config": { "font": "Noto Sans" }`.

**Both:** `Date` in a renderer returns a fixed time in UTC, not the server's
clock or time zone. A time axis formats dates in UTC. Put dates into the spec as
values, and pre-format labels when the time zone matters.

## When an island fails

| Kind | When | Ruby | Rust |
|---|---|---|---|
| `invalid-island` | No `data-renderer`, a bad size, JSON that doesn't parse, no `</script>` | `Surimono::RenderError` | `Error::InvalidIsland` |
| `unknown-renderer` | The renderer isn't built in, registered or enabled | `Surimono::RenderError` | `Error::UnknownRenderer` |
| `memory-limit` | The script used more heap than the memory limit | `Surimono::MemoryLimitError` | `Error::MemoryLimit` |
| `time-limit` | The script ran longer than the time limit | `Surimono::TimeLimitError` | `Error::TimeLimit` |
| `stack-overflow` | The script recursed deeper than the stack limit | `Surimono::StackOverflowError` | `Error::StackOverflow` |
| `javascript` | The script threw | `Surimono::JavaScriptError` (with `js_stack`) | `Error::JavaScript` (with `stack`) |
| `render` | The renderer's result isn't usable SVG | `Surimono::RenderError` | `Error::Render` |
| `html` | The document is too ambiguous to rewrite | `Surimono::RenderError` | `Error::Html` |

Every island error names the island: in Ruby `island_index` (from 0),
`island_id` and `renderer`; in Rust `Error::island()`. `kind` is the stable
name in the first column.

There are two policies:

- **Fail** (the default): the first failed island ends the render with its
  error. A document that silently misses a chart is usually worse than no
  document.
- **Placeholder**: a failed island becomes an empty element and the rest of the
  document renders.

  ```html
  <div class="surimono-island surimono-island-error chart" id="sales"
       data-renderer="echarts" data-error="time-limit"><!-- surimono: … --></div>
  ```

  The comment holds the error message. The placeholder has no size, so give
  `.surimono-island-error` a size or a visible style if the gap should show.

Choose it with `error_policy: :placeholder` in Ruby or
`Options::error_policy(ErrorPolicy::Placeholder)` in Rust.

## Your own renderer

Any JavaScript that turns JSON into an SVG string can be a renderer. A bundle is
a classic script (not an ES module) that defines a global function:

```js
entry(specJson, ctx)  // → SVG string, or a Promise of one
```

- `specJson` is the island's text, already checked to be JSON. Parse it with
  `JSON.parse`.
- `ctx.width` and `ctx.height` are the island's size in CSS pixels.
- `ctx.measureText(text, cssFont)` measures text with your registered fonts and
  returns the fields of a canvas `TextMetrics`: `width`,
  `actualBoundingBoxLeft`, `actualBoundingBoxRight`,
  `actualBoundingBoxAscent`, `actualBoundingBoxDescent`,
  `fontBoundingBoxAscent` and `fontBoundingBoxDescent`. `cssFont` is a CSS
  `font` shorthand such as `"bold 12px 'Noto Sans'"`, default
  `"10px sans-serif"`.
- If it returns a Promise, the Promise must settle without timers, within the
  time limit.

The function can have any name; you give it when you register the bundle.

An example, a sparkline with a measured label:

```js
// sparkline.js
globalThis.render = function render(specJson, ctx) {
  const { values, label = "" } = JSON.parse(specJson);
  const font = "12px 'Noto Sans'";
  const text = label ? ctx.measureText(label, font) : { width: 0, actualBoundingBoxAscent: 0 };
  const plotWidth = ctx.width - (label ? text.width + 6 : 0);
  const min = Math.min(...values), max = Math.max(...values);
  const x = (i) => (i * plotWidth) / Math.max(values.length - 1, 1);
  const y = (v) => ctx.height - 1 - ((v - min) / (max - min || 1)) * (ctx.height - 2);
  const points = values.map((v, i) => `${x(i).toFixed(2)},${y(v).toFixed(2)}`).join(" ");
  const escaped = label.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}">` +
    `<polyline fill="none" stroke="#2f6fb0" stroke-width="1.5" points="${points}"/>` +
    `<text x="${ctx.width}" y="${(ctx.height + text.actualBoundingBoxAscent) / 2}" ` +
    `text-anchor="end" font-family="Noto Sans" font-size="12">${escaped}</text></svg>`;
};
```

Register it under a name and use that name in `data-renderer`:

```ruby
renderer = Surimono::Renderer.new(
  fonts: FONTS,
  bundles: { "sparkline" => { source: File.read("app/pdf/sparkline.js"), entry: "render" } },
)
```

```rust
let options = Options::new()
    .font(font_bytes)
    .bundle("sparkline", std::fs::read_to_string("pdf/sparkline.js")?, "render");
```

```html
<script type="application/vnd.surimono+json" data-renderer="sparkline" data-width="200" data-height="24">
  { "values": [3, 5, 4, 8, 6, 9], "label": "+12%" }
</script>
```

What a bundle can rely on, and what it can't:

- **The JavaScript language only.** There is no `window`, `document`,
  `console`, `setTimeout`, `fetch`, `require` or `import`. A library that
  expects a browser or Node.js needs its server-side or headless mode, plus
  small stubs you define at the top of the bundle (for example
  `globalThis.console = { log() {}, warn() {}, error() {} };`).
- **Bundle it first.** Use a bundler that writes one classic script, such as
  `esbuild sparkline.js --bundle --format=iife --outfile=dist/sparkline.js`,
  and assign the entry to `globalThis` as above, because `iife` output keeps
  top-level names private.
- **It runs once at registration**, under the same limits, to check that the
  entry exists. It is compiled to bytecode once, and every island then starts
  from a fresh context. Nothing one island sets is visible to the next.
- **The same limits apply** as to the built-in renderers.
- **The output is cleaned** before it is inlined: `<script>` elements, event
  handler attributes and external URLs are removed, every `id` gets a per-island
  prefix (references to it follow), font families are rewritten to your
  registered fonts, and text is drawn as outlines (below). Return plain SVG
  with `<text>` elements and you won't notice.
- **Determinism is yours to keep.** `Math.random` is seeded and `Date` is fixed,
  so a bundle that only uses the language gives the same output for the same
  spec.

## Fonts in islands

Renderers measure text with `measureText`, which uses only the fonts you
registered.

- A font list resolves to the registered families it names, in order. Generic
  names (`sans-serif`, `serif`, `monospace`) resolve to the first registered
  family. Names that aren't registered are skipped, and a list with no
  registered name uses the first registered family.
- Every other registered family follows as a per-character fallback, in the
  order you registered the fonts. So Japanese text in a `"Noto Sans"` label
  falls back to Noto Sans JP if you registered it, as in a browser.
- surimono writes that resolved list into the SVG's `font-family`.

### Chart text is drawn as outlines

By default, surimono then draws every piece of text in the island's SVG as
vector paths, glyph by glyph, from the same fonts and the same shaping that
`measureText` used. The SVG your PDF engine receives has no `<text>` left to
lay out.

Why: fulgur 0.40 prints `<svg>` text with the machine's system fonts, not the
fonts registered with it. Real text would print in a different font from the one
it was measured with, so labels could overflow or clip, and the PDF would change
from one server to the next. Outlines print exactly as measured, on any machine.

What it costs:

- Chart text can't be selected, searched or copied in the PDF.
- The original string is kept in the SVG for accessibility
  (`aria-label` / `<title>`); what reaches the PDF depends on the engine.
  <!-- confirm: exact attribute/element when the gfx outline change lands -->
- The SVG, and so the PDF, gets larger, since every label becomes path data.

To keep real `<text>` instead, pass `text: :keep` in Ruby or `TextMode::Keep` in
Rust. Do this only when your PDF engine prints SVG text with your registered
fonts. With fulgur 0.40 it doesn't: labels print in a system font, may not fit,
and differ between servers.

Either way, register the same font files with your PDF engine for the page's
own text. The [Rails](rails.md#fonts) and [Rust](rust.md#fonts) guides show how.
