# Migrating from Grover (chart-only documents)

Many Rails apps run Chrome through Grover for one reason: the PDF has a chart
in it, and the chart is drawn by JavaScript. surimono 0.1 draws those charts
without a browser, so documents whose only scripts are charts can move to
surimono and a browserless engine such as fulgur.

This guide is for those documents. Check first whether yours is one of them.

<!-- bench: grover-vs-surimono -->
_The benchmark against Grover (time and peak memory, cold and warm) will be
linked here once `bench/` has results._

## What still needs Chrome in v0.1

Keep Grover, for now, for any document that needs one of these:

- **The page's own scripts.** surimono 0.1 runs island renderers only. Scripts
  in the page are passed through and never run: anything that builds or changes
  the DOM, Stimulus or Alpine.js controllers, KaTeX auto-render, highlight.js,
  dates formatted in the browser, D3 code that appends elements. Page scripts on
  a real DOM are planned for v0.2.
- **Scripts that measure layout.** `getBoundingClientRect`, `offsetWidth`,
  `getBBox`, `getComputedTextLength`, sizing a chart to its container, or text
  fitting. Layout queries are planned for v0.3.
- **Canvas.** Chart.js and other libraries that draw on `<canvas>`. v0.1 has
  ECharts and Vega, which draw SVG. Canvas recorded as vectors, in the scope
  Chart.js uses, is planned for v0.3.
- **The network.** Neither surimono nor fulgur fetches anything: no web fonts,
  scripts, stylesheets or images from URLs, no `fetch` for chart data, and no
  rendering of a live URL (`Grover.new("https://…")`).
- **Chrome features.** `wait_for_selector`, `execute_script`, screenshots
  (`to_png`, `to_jpeg`), cookies and authentication for URLs, and
  `header_template`/`footer_template` as HTML.
- **Selectable chart text.** In Chrome's PDF, text in an SVG chart can be
  selected and searched. surimono draws chart text as outlines by default, so it
  can't (see [why](islands.md#chart-text-is-drawn-as-outlines)).

fulgur is also not Chrome. It lays out CSS with Blitz rather than Chrome's
engine, so page layout can differ in places. [docs/COMPAT.md](../COMPAT.md) lists the differences we have
found. Compare a few real documents before you switch a template over.

## 1. Find the charts

List the templates Grover prints and what scripts each one has:

```sh
grep -rln --include='*.erb' -e '<script' -e '<canvas' -e 'javascript_include_tag' app/views
```

Sort them into three groups:

- **No scripts:** move them to fulgur alone. surimono passes them through
  unchanged.
- **Charts only:** ECharts or Vega/Vega-Lite drawn from data the server already
  has. These are what this guide moves.
- **Anything else:** keep them on Grover for now.

A chart drawn with another library (Chart.js, Highcharts, ApexCharts) has to be
rewritten as an ECharts or Vega-Lite spec first. Both cover the usual bar, line,
area, pie and scatter charts.

## 2. Turn each chart into an island

A typical Grover template draws the chart in the page:

```erb
<div id="sales" style="width: 680px; height: 320px"></div>
<%= javascript_include_tag "echarts.min" %>
<script>
  const chart = echarts.init(document.getElementById("sales"), null, { renderer: "svg" });
  chart.setOption({
    animation: false,
    xAxis: { type: "category", data: <%= raw @quarters.to_json %> },
    yAxis: { type: "value", axisLabel: { formatter: (v) => v.toLocaleString() + " 円" } },
    series: [{ type: "bar", data: <%= raw @sales.to_json %> }],
  });
</script>
```

With surimono the option becomes data in an island marker. The `surimono_island`
helper is in the [Rails guide](rails.md#the-view):

```erb
<%= surimono_island "echarts", {
      xAxis: { type: "category", data: @quarters },
      yAxis: { type: "value", axisLabel: { formatter: "{value} 円" } },
      series: [{ type: "bar", data: @sales }],
    }, width: 680, height: 320, id: "sales" %>
```

What changed:

- **The option is JSON now**, built in Ruby. Functions can't be part of it. Use
  the string templates ECharts offers (`"{value} 円"`), or format values in Ruby
  (`ActiveSupport::NumberHelper.number_to_delimited`) and pass the strings.
- **The size moves** from the `div`'s style to `width:` and `height:`, in CSS
  pixels.
- **No `animation: false` needed.** surimono turns animation off.
- **No `<script>` tags for the library.** ECharts and Vega are built into the
  gem.

## 3. Register your fonts

Chrome found fonts on the machine or loaded them from `@font-face` URLs.
surimono and fulgur only use font files you give them. List them once and pass
the same files to both, and name those families in your CSS. The
[Rails guide](rails.md#fonts) shows how, and why `font-family: sans-serif` on its
own is not enough.

## 4. Replace the Grover call

Before:

```ruby
html = render_to_string(:show, layout: "pdf")
pdf = Grover.new(html, format: "A4", margin: { top: "15mm", bottom: "15mm", left: "15mm", right: "15mm" },
                       print_background: true, wait_until: "networkidle0").to_pdf
send_data pdf, type: "application/pdf", disposition: "inline"
```

After, with the `PdfRenderer` from the [Rails guide](rails.md#one-renderer-per-process):

```ruby
html = render_to_string(:show, layout: "pdf")
pdf = PdfRenderer.render(html)   # islands to SVG, then fulgur
send_data pdf.to_s, type: "application/pdf", disposition: "inline"
```

Grover's options map like this:

| Grover | With surimono and fulgur |
|---|---|
| `format: "A4"` | `@page { size: A4; }` in the layout's CSS, or `Fulgur::Engine.new(page_size: :a4)` |
| `landscape: true` | `Fulgur::Engine.new(landscape: true)` |
| `margin: { top: "15mm", … }` | `@page { margin: 15mm; }`, or `margin: Fulgur::Margin.new(…)` in points |
| `display_header_footer`, `header_template`, `footer_template` | CSS page margin boxes: `@page { @bottom-right { content: "Page " counter(page) " / " counter(pages); } }` |
| `wait_until`, `wait_for_selector` | Not needed. There is nothing to wait for: islands are drawn before printing. |
| `display_url` (base URL for assets) | fulgur doesn't fetch. Register stylesheets with `assets.add_css_file` and images with `assets.add_image_file` (see fulgur's README). |
| `execute_script` | Not available in v0.1 |

Options passed to `Fulgur::Engine.new` win over the CSS `@page` rule.

## 5. Compare, then switch

For each template you move:

1. Print a few real documents both ways and compare them side by side. Look at
   the charts' fonts and labels, page breaks, tables, and anything positioned.
2. Add a test that renders the islands twice and compares the HTML. The output
   is deterministic, so any difference is a bug.
3. Switch the template over behind a flag, and keep the Grover path until you
   have printed real traffic with the new one.

When a chart fails, the error names the island: `e.kind`, `e.island_index`,
`e.island_id` and `e.renderer`. The [islands guide](islands.md#when-an-island-fails)
lists the error kinds.

## Ferrum and WickedPDF

The same steps apply to Ferrum (Chrome over CDP) and to WickedPDF
(wkhtmltopdf). wkhtmltopdf runs an old WebKit, so if the charts there are
already static images or server-side SVG, there may be no script to move at
all: try fulgur on its own first.
