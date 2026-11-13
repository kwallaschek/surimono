# surimono

Run a document's JavaScript without a browser, so browserless HTML-to-PDF
engines can print it.

surimono is a Rust library, a Ruby gem and a command-line tool. It runs the scripts a document needs in
a sandboxed QuickJS runtime and returns HTML that an engine such as
[fulgur](https://github.com/fulgur-rs/fulgur) can print. It does this in two
ways: it finds chart *islands* in your HTML, draws them as SVG with ECharts or
Vega and inlines the SVG, and on request it runs the page's own scripts against
a DOM with no layout.

> **Status:** not released. There is no gem on RubyGems and no crate on
> crates.io, so `gem install surimono` doesn't work, and the first release is
> planned for v0.3. Until then, use the gem, the crates and the command-line
> tool from a clone of this repository (see [Install](#install)). This README
> describes what `main` does today.

## Why

Browserless PDF engines (fulgur, ironpress, rustyprint) don't run scripts, so
one chart in an invoice or report sends you back to Chrome through Grover,
Ferrum or a Gotenberg sidecar, with Chrome's memory use and start-up time.
surimono runs only the JavaScript the document needs, without a browser, and
then gets out of the way.

<!-- bench: grover-vs-surimono -->
Median time and peak memory per document for a six-page report with six charts,
cold (a new process) and warm (a long-lived process):

| | Grover (Chrome) | Grover (chrome-headless-shell) | surimono + fulgur, Ruby | surimono + fulgur, Rust |
|---|---|---|---|---|
| Time, cold | 1,698 ms | 1,284 ms | 694 ms | 570 ms |
| Time, warm | 1,268 ms | 1,009 ms | 538 ms | 538 ms |
| Peak memory, cold | 589 MB | 383 MB | 101 MB | 74 MB |
| Peak memory, warm | 712 MB | 494 MB | 128 MB | 87 MB |

In the ratios here, cold comes first. Against Grover on Chrome, that is 2.4× and
2.4× faster in Ruby (Rust: 3.0× and 2.4×), with 5.8× and 5.6× lower peak memory
(Rust: 8.0× and 8.2×). The project's targets are at least 2× and 5×. The gap to
chrome-headless-shell, the leanest Chrome a Grover user can run, is smaller:
1.8× and 1.9× faster with 3.8× and 3.8× lower peak memory in Ruby, below both
targets, and 2.3× and 1.9× with 5.2× and 5.7× in Rust.

Peak memory is PSS summed over the process tree. These numbers are one run on
GitHub Actions, at commit `15b530e0`, and times vary from run to run
with the machine. The report is shaped around fulgur 0.40's layout gaps, so it
is not a typical Rails template. See the [full results](bench/results/15b530e0/summary.md),
whose memory verdicts use the earlier 10× target, and the
[method](bench/README.md). The report's scripts are all chart islands: no
benchmark of a page's own scripts is committed yet, so these numbers say nothing
about them.

## What it does today

- **Islands.** It replaces each
  `<script type="application/vnd.surimono+json" data-renderer="…">` marker in
  your HTML with the SVG its renderer draws. Everything else in the document
  stays byte for byte as it was.
- **Page scripts.** `render_page` runs the document's own scripts as a browser
  would, on a virtual clock until the page settles, and returns the HTML and a
  report of what went wrong. See [Page scripts](#page-scripts).
- **Built-in renderers.** `echarts` (Apache ECharts 6.1.0, server-side SVG),
  `vega-lite` (Vega-Lite 6.5.0) and `vega` (Vega 6.4.0). They ship as
  precompiled QuickJS bytecode, so a new process skips parsing and compiling
  the libraries. Sources and licences are listed in
  [THIRD_PARTY.md](THIRD_PARTY.md).
- **Bring your own bundle.** Any script that defines a function taking the
  island's JSON and returning an SVG string can be a renderer.
- **Chart text in your fonts.** Chart labels are measured and drawn with the
  font files you pass in, so they fit and look the same on any server.
- **A sandbox.** Every script runs under memory, time and stack limits, with no
  filesystem or network access unless you pass a loader for a page's files.
- **Ruby, Rust and a CLI.** The gem, the crate and the `surimono` command take
  the same options and give the same bytes for the same input.
- **Safe to share and to fork.** One renderer serves many threads, and a
  renderer built before Puma, Unicorn or Resque forks works in every child.

## What it is not

- **Not a browser.** Nothing is laid out, and much of the web platform is
  missing. See [Page scripts](#page-scripts) for what is.
- **Not for websites.** It doesn't fetch pages, styles, images or data, and it
  has no network access. A page gets only the files you hand it.
- **Not a PDF engine.** Pair it with fulgur or any engine that prints HTML with
  inline SVG.

## Install

Nothing is released yet, so everything here installs from this repository. The
first release is planned for v0.3.

**Ruby** (CRuby 3.2 to 4.0): there is no gem on RubyGems, so
`gem install surimono` doesn't work. Build the gem from a clone instead. That
needs Rust 1.89 or later, a C compiler and libclang (rb-sys generates its Ruby
bindings with bindgen).

```sh
git clone https://github.com/kwallaschek/surimono
cd surimono/bindings/ruby
bundle install
bundle exec rake compile   # builds the extension; expect several minutes
```

Then point your app's Gemfile at the clone:

```ruby
# Gemfile
gem "surimono", path: "/path/to/surimono/bindings/ruby"
gem "fulgur", "0.40.0"   # or another browserless PDF engine
```

Bundler doesn't compile a gem from `path:`, so run `rake compile` first, and
again after you pull. fulgur's gem 0.40.0 supports Ruby 3.1 to 3.4, so the pair
runs on Ruby 3.2 to 3.4 today.

*Planned for the first release, not available today:* `gem "surimono"` from
RubyGems, with precompiled gems for x86_64 and aarch64 Linux (glibc) and x86_64
and arm64 macOS.

**Rust** (1.89 or later; a C compiler builds QuickJS):

The Rust crates aren't on crates.io yet. surimono needs a patched QuickJS, and
a crates.io release can't carry the patch until the fixes are upstream or
published under crate names of our own. Until then, use the crates from this
repository. A git dependency works if your own `Cargo.toml` also patches
`rquickjs-sys`, because a dependency's `[patch]` never reaches your build:

```toml
[dependencies]
surimono = { git = "https://github.com/kwallaschek/surimono", branch = "main" }

[patch.crates-io]
rquickjs-sys = { git = "https://github.com/kwallaschek/surimono", branch = "main" }
```

Without the `[patch.crates-io]` entry the build fails at link time with
`undefined reference to surimono_quickjs_patch_level`. Cargo finds
`rquickjs-sys` in the repository's `vendor/` folder. Keep both entries on the
same branch or `rev` so they come from one commit.

ECharts and Vega are Cargo features, both on by default: `echarts` adds the
`echarts` renderer, `vega` adds `vega` and `vega-lite`. To leave Vega out, add
`default-features = false, features = ["echarts"]` to the `surimono` line.

**Command line:** build it from a clone of the repository, with Rust 1.89 or
later and a C compiler, as for the crates.

```sh
git clone https://github.com/kwallaschek/surimono
cd surimono
cargo install --locked --path crates/cli   # installs the `surimono` command
```

`cargo install --git …` doesn't work: it ignores the repository's `[patch]`,
so the build fails at link time.

## Rails example

The view puts the chart's spec into an island marker:

```erb
<%# app/views/reports/show.html.erb %>
<h1>Sales by quarter</h1>
<script type="application/vnd.surimono+json" data-renderer="echarts" data-width="640" data-height="320">
  <%= raw json_escape(@chart.to_json) %>
</script>
```

The controller renders the view, draws the islands, and prints the result with
fulgur. Both get the same font files: surimono draws the chart text with them,
and fulgur prints the page's own text with them.

```ruby
class ReportsController < ApplicationController
  FONTS = %w[NotoSans-Regular.ttf NotoSansJP-Regular.otf].map { |f| Rails.root.join("app/fonts", f).to_s }

  def show
    @chart = { xAxis: { data: %w[Q1 Q2 Q3 Q4] }, yAxis: {}, series: [{ type: "bar", data: [120, 200, 150, 80] }] }
    html = Surimono.render(render_to_string(:show, layout: "pdf"), fonts: FONTS)
    assets = Fulgur::AssetBundle.new
    FONTS.each { |path| assets.add_font_file(path) }
    pdf = Fulgur::Engine.new(page_size: :a4, assets: assets).render_html(html)
    send_data pdf.to_s, type: "application/pdf", disposition: "inline"
  end
end
```

Use `raw json_escape(…)`, not plain `<%= … %>`: ERB would turn `"` into `&quot;`
and the island would no longer be JSON. In production, build one
`Surimono::Renderer` and one fulgur engine and reuse them, and render large
documents in a background job. The [Rails guide](docs/guide/rails.md) shows how.

## Rust example

```rust
use surimono::{Options, Renderer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut options, mut assets) = (Options::new(), fulgur::AssetBundle::new());
    for path in ["fonts/NotoSans-Regular.ttf", "fonts/NotoSansJP-Regular.otf"] {
        let bytes = std::fs::read(path)?;
        options = options.font(bytes.clone()); // chart text
        assets.add_font_bytes(bytes)?;         // page text
    }
    let renderer = Renderer::new(options)?; // build once, reuse: it is Send + Sync
    let html = renderer.render_islands(&std::fs::read_to_string("report.html")?)?;
    let engine = fulgur::Engine::builder().assets(assets).system_fonts(false).build();
    std::fs::write("report.pdf", engine.render(&html)?)?;
    Ok(())
}
```

More in the [Rust guide](docs/guide/rust.md).

## Page scripts

When a template's own scripts build the page, ask for them to run. `render` and
`render_islands` never run them.

```ruby
page = Surimono.render_page(html, fonts: FONTS)
page.report.uncaught   # exceptions nothing caught; the page goes on, as in a browser
pdf = Fulgur::Engine.new(page_size: :a4, assets: assets).render_html(page.html)
```

```rust
let page = renderer.render_page(&html)?;
eprintln!("{} uncaught, {:?}", page.report.uncaught, page.report.settled);
let pdf = engine.render(&page.html)?;
```

- **Timers run on a virtual clock**, so a `setTimeout` of a second costs no real
  time, and a page gets 10 seconds of virtual time to settle.
- **A page's scripts share one set of limits**, not one per script. A limit
  raises and gives no page; an exception in a page goes in the report.
- **No files by default.** An external script needs `resources:`, or a `loader:`
  with `allow:` (a `ResourceLoader` in Rust). The URL reaches it as written, with
  no base applied. A stylesheet comes out as `<style>`, and a `url()` or `@import`
  in it isn't followed.
- **The output is serialized**, so it isn't byte-identical to the input:
  `<noscript>` and comments are always gone, and `<script>` too unless you pass
  `keep_scripts`.
- **The output is printer-safe** by default: a block `<svg>`'s size, the
  colour of a border on one side and a newline ending a `<pre>` are written in
  a form fulgur 0.40 prints the way Chrome prints the page, only where Chrome
  prints the result exactly as before. The HTML then differs from the page's
  DOM by one `<style>` in `<head>` and by those newlines. `printer_safe: false`
  (Ruby), `Options::printer_safe(false)` (Rust) or `--no-printer-safe` gives
  the DOM as it is.

The [Rails guide](docs/guide/rails.md#page-scripts) and the
[Rust guide](docs/guide/rust.md#page-scripts) cover all of it, including how to
print pages you don't trust.

How much of the web platform works, in Web Platform Tests subtests, against what
Chrome passes in the same run (from [docs/COMPAT.md](docs/COMPAT.md), surimono
at `221530524e`, WPT at `8b4b06ff53`). All four phase 2 targets are met:

| Area | Passed | Of | Rate | Phase 2 target |
|---|---|---|---|---|
| Timers and microtask queuing | 31 | 34 | 91.2% | 90%, met |
| `dom/nodes` | 14,168 | 15,267 | 92.8% | 70%, met |
| `dom/events` | 566 | 748 | 75.7% | 70%, met (66.7% of wpt.fyi's 848) |
| `domparsing` | 566 | 909 | 62.3% | 60%, met |

wpt.fyi, running Chrome 156 at the same WPT commit, counts more subtests that
Chrome passes (100 more in `dom/events`, 37 in the other three together). On its
count `dom/events` is below the target; the other three stay above it.
COMPAT.md has both tables.

Six real-world pages, built with Alpine.js, highlight.js, D3, KaTeX and marked,
were printed through `render_page` and stock fulgur 0.40.0 and compared with
Chrome's print: 4 of 6 pass with printer-safe output (`alpine-invoice`,
`d3-charts`, `highlightjs-code`, `noscript-message`), 2 without it. The other
2, `katex-autorender` and `marked-report`, print the same as fulgur printing
Chrome's own DOM, so those failures are fulgur's layout, not the page's
scripts. The [committed results](fixtures/checkpoint-3/index.md) are of the
output without printer-safe rewrites.

What doesn't work yet:

- **Layout queries answer zeros.** `getBoundingClientRect`, `offsetWidth` and
  `getComputedStyle` give zeros and empty strings, or fail the render if you ask
  for that (`layout_queries: :throw`), so a script that sizes things from them
  may draw at size 0. Real answers are planned for v0.3.
- **No canvas drawing.** A `<canvas>` has no `getContext`. Canvas as vectors, in
  the scope Chart.js uses, is planned for v0.3.
- **A script a script inserts doesn't run**, and `type="module"` scripts don't
  run.
- **No `fetch`, `XMLHttpRequest`, `localStorage`, `Worker`, `WebSocket`,
  `matchMedia`, `ResizeObserver`, `IntersectionObserver`, `Intl` or shadow
  DOM**, and no `document.write`.
- **Some members answer as if there were no browser around the page.**
  `innerText` sees no stylesheet until phase 3, only `hidden`, an inline
  `display: none` and each element's default display. `document.cookie` is
  empty and a write does nothing. `:focus` doesn't match. `customElements` is
  a registry only: a defined element is never constructed or upgraded.
  `navigator` is fixed values, the same on every machine.
- **Images aren't loaded for a page**: no `load` or `error` event fires for one,
  `new Image()` included. `src` stays for the PDF engine.

## Command-line example

```sh
surimono render report.html -o report.out.html \
  --font fonts/NotoSans-Regular.ttf --font fonts/NotoSansJP-Regular.otf
```

`-` reads standard input or writes standard output, and `-o` defaults to
standard output. The output file is written only after the render succeeded.
`surimono --help` lists every option (limits, `--seed`, `--now-ms`,
`--utc-offset-minutes`, `--error-policy`, `--text`, `--bundle`,
`--renderers`, `--max-islands`, `--pool-size`).

`--scripts` runs the page's own scripts first. A page's files come from
`--resource`, with the URL as the page writes it, and what went wrong in the page
goes to standard error without failing the render:

```sh
surimono render page.html -o page.out.html --font fonts/NotoSans-Regular.ttf \
  --scripts --resource /assets/invoice.js=app/assets/invoice.js
```

`--keep-scripts`, `--no-printer-safe`, `--layout-queries` and `--output-mb` go
with it. Exit codes tell a script what to fix:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | An island failed (bad marker, unknown renderer, script error, unusable SVG), more islands than `--max-islands`, a document that can't be rewritten, a page that couldn't run, or a layout query under `--layout-queries throw` |
| 2 | Usage error: an unknown option or a bad value |
| 3 | A script hit its memory, time or stack limit, or a page a limit of the DOM |
| 4 | Invalid configuration: no font, a font or bundle that can't be used, limits out of range |
| 5 | A file couldn't be read or written |

## Island markers

```html
<script type="application/vnd.surimono+json" id="sales" class="chart"
        data-renderer="echarts" data-width="640" data-height="320">
  { "xAxis": { "data": ["Q1", "Q2"] }, "yAxis": {}, "series": [{ "type": "bar", "data": [3, 5] }] }
</script>
```

| Attribute | Required | Meaning |
|---|---|---|
| `type` | yes | `application/vnd.surimono+json` |
| `data-renderer` | yes | `echarts`, `vega-lite`, `vega`, or the name of your own bundle |
| `data-width`, `data-height` | no | Size in CSS pixels, a whole number from 1 to 16384. Default 600 × 400. |
| `id`, `class` | no | Kept on the output element |
| `style`, `title`, `lang`, `dir`, `role`, `aria-*` | no | Kept on the output element; `style` comes after the island's size |

The element's text is the renderer's spec as JSON: an ECharts option object, a
Vega-Lite spec or a Vega spec. The marker becomes a box of the island's size
that clips anything drawn outside it:

```html
<div class="surimono-island chart" id="sales" data-renderer="echarts"
     style="width:640px;height:320px;overflow:hidden"><svg …>…</svg></div>
```

- Put markers where a `<div>` is allowed, not inside `<p>`, `<span>` or `<a>`.
- Inside the JSON, write `<` as `\u003c`, so `</script>` in a string can't end
  the marker. Rails' `json_escape` does this for you.
- If an island fails, the whole render fails with an error that names the
  island. With `error_policy: :placeholder` (Ruby), `ErrorPolicy::Placeholder`
  (Rust) or `--error-policy placeholder` (CLI), a failed island becomes an
  empty box of the same size,
  `<div class="surimono-island surimono-island-error" data-error="…">`, and the
  rest of the document renders.

The [islands guide](docs/guide/islands.md) covers sizing, the renderers, your
own bundles and fonts.

## Sandbox and determinism

Every island runs in a fresh QuickJS runtime, and every page gets new script
state, with these limits:

| Limit | Default | Ruby option | Error when exceeded |
|---|---|---|---|
| JavaScript heap | 64 MB | `limits: { memory_mb: }` | `Surimono::MemoryLimitError` / `Error::MemoryLimit`, `Error::PageMemoryLimit` |
| Running time | 2 s | `limits: { time_ms: }` | `Surimono::TimeLimitError` / `Error::TimeLimit`, `Error::PageTimeLimit` |
| Stack | 512 KB | `limits: { stack_kb: }` | `Surimono::StackOverflowError` / `Error::StackOverflow`, `Error::PageStackOverflow` |
| A page's output | 64 MB | `limits: { output_mb: }` | `Surimono::DomLimitError` / `Error::PageDomLimit` |

A script that hits a limit stops with that typed error. It doesn't crash or
hang your process. An island's script gets the JavaScript language and nothing
else: no filesystem, network, module imports, DOM, timers or `console`. The
built-in renderers bring the few stubs their libraries need. A page's scripts
also get a DOM and timers, and still no filesystem or network.

The limits apply to each island, so a document with N islands can take up to
N × the time limit. A page's scripts share one time limit; its parse has another
of the same length, and all that follows the scripts a third. Two things bound a
whole document:

- **`max_islands`** (default 1,000): a document with more islands fails with
  `too-many-islands` before any island runs.
- **Cancellation.** In Ruby, `Timeout.timeout`, `Rack::Timeout`,
  `Thread#raise` or a signal handler that raises in the rendering thread stops
  a render, its running island or page script included. In Rust,
  `Renderer::render_islands_cancellable` and
  `Renderer::render_page_cancellable` take an `AtomicBool` to the same effect. Loading a bundle is the exception: it isn't interrupted, and a cancel
  takes effect when it ends. Precompiled built-ins load in place. A built-in
  compiled from source on first use, or your own bundle compiled at a forked
  process's first render, can take up to twice the time limit (one limit to
  compile, one to run it once), plus any wait for a free runtime.

The SVG that comes back is cleaned before it is inlined: no `<script>`, event
handlers, style sheets, `<foreignObject>`, animation or external URLs, and
every `id` prefixed per island, so no chart can reach the page or another
chart. Its text is then drawn as outlines (see [Fonts](#fonts)).

The same HTML and options give the same bytes on every run and every machine:

- `Math.random` is seeded (the `seed` option, default 0).
- `Math.sin`, `Math.pow` and the other `Math` functions are correctly rounded,
  so they don't depend on the machine's C library.
- `Date` sees a fixed "now", 2026-01-01T00:00:00Z unless you set `now:`
  (Ruby) or `Options::now_ms` (Rust), and local time is UTC unless you set
  `utc_offset_minutes:` (540 for Japan). The server's clock and time zone are
  never read.
- Text is measured only with the fonts you pass in. System fonts are never read.
- A page's timers run on a virtual clock, and its files come from you.

Your PDF engine has no limits of its own. Print pages you don't trust in a
process of your own, with its own timeout and stack size: the
[Rails guide](docs/guide/rails.md#printing-pages-you-dont-trust) and the
[Rust guide](docs/guide/rust.md#printing-pages-you-dont-trust) show how.

## Fonts

Pass the same font files to surimono and to your PDF engine.

- **Chart text becomes outlines.** By default surimono draws every label in an
  island as vector paths, using the font files you passed. fulgur 0.40 prints
  `<svg>` text with the machine's system fonts rather than the fonts you
  register, so labels would otherwise print in a different font from the one
  they were measured with, and differ from server to server. As outlines they
  look exactly as measured, everywhere. The cost: chart text can't be selected,
  searched or copied in the PDF. Each label becomes a
  `<g role="img" aria-label="…">`, so the words stay in the SVG for
  accessibility.
- **Keeping real text is an option** (`text: :keep` in Ruby,
  `TextMode::Keep` in Rust). Do this only if your PDF engine prints SVG text
  with your registered fonts. With fulgur 0.40 it doesn't, so labels print in
  a system font and may overflow or clip.
- **Name your fonts in page CSS.** surimono doesn't touch the rest of the page,
  and your PDF engine prints its text. Write
  `font-family: "Noto Sans", "Noto Sans JP"` with the family names inside the
  font files, not `sans-serif` alone. In Rust with fulgur's system fonts off,
  text set in a generic family alone prints blank. With fulgur's Ruby gem 0.40.0,
  which can't turn system fonts off, it prints in whatever font the server has,
  so it differs between machines.
- **TrueType, OpenType and collections only.** WOFF and WOFF2 are rejected.
  Convert them to TTF or OTF first.
- **Order matters.** Characters a font lacks fall back to the next font in the
  order you passed them, for example Japanese after a Latin font. List fonts
  explicitly rather than with a glob.

## Current limits

- **Scripts can't measure layout.** `getBoundingClientRect` and friends answer
  zeros or fail the render, and real answers are planned for v0.3. Documents that
  need them still need Chrome for now. [Page scripts](#page-scripts) lists what
  else a page can't do yet.
- **Specs are JSON.** You can't pass JavaScript functions, such as an ECharts
  `formatter` callback. Use the string templates the libraries offer
  (`"formatter": "{value} 円"`), or write your own bundle.
- **No network.** A Vega spec that loads data from a URL fails. Put the data in
  the spec (`"values": [...]`).
- **Chart text isn't text.** By default, chart labels are vector outlines in
  the PDF, so they can't be selected, searched or copied, and screen readers
  get only what the PDF engine makes of the string kept in the SVG. Keeping
  real text (`text: :keep`) prints it in a system font with fulgur 0.40.
- **Static output.** Animations are off and the chart is drawn in its final
  state. Tooltips and hover states don't exist in a PDF.
- **The time limit is per island.** A document with many islands can take that
  many times as long; cap it with `max_islands` and cancel from outside (see
  [Sandbox and determinism](#sandbox-and-determinism)). The same goes for islands
  in a page.
- **Vega data and size.** Vega and Vega-Lite specs keep their own `width` and
  `height` (a Vega-Lite `"container"` size means the island's). Images, links
  and transforms that draw to a raster canvas (label placement, word clouds,
  heatmap images) aren't supported.
- **QuickJS is not V8.** It is an interpreter, so a dense chart may need a time
  limit above the default 2 s. The [benchmark](#why) times whole documents, so
  it doesn't show how a single chart compares with Chrome.
- **Mermaid is not included.**
- **Platforms.** CI covers Linux (glibc) and macOS; there is no Windows or musl
  support yet.
- **Everything comes from the repository.** Nothing is on RubyGems or crates.io
  yet; see [Install](#install).

## Compatibility

[docs/COMPAT.md](docs/COMPAT.md) lists what is known to work and what is known
not to, generated from the test results. Every gap is listed there rather than
hidden.

## Roadmap

v0.3 is planned as the first release and will carry all three phases.

| Phase | Adds | State |
|---|---|---|
| 1 | Chart islands | On `main`; the `v0.1.0` tag marks it, but it was not released |
| 2 | The page's own scripts on a real DOM (blitz-dom), on a virtual clock | On `main`, in progress; all four WPT targets met (see [Page scripts](#page-scripts)) |
| 3 | Layout queries that measure as Chrome does; Canvas2D as vectors | Planned |

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT licence](LICENSE-MIT), at your option. Bundled JavaScript libraries keep
their own licences, listed in [THIRD_PARTY.md](THIRD_PARTY.md). Every gem
includes `THIRD_PARTY_LICENSES.txt` with the licences and notices of all the
Rust crates and JavaScript libraries built into it; include it when you
redistribute the gem or a binary built from surimono.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 licence, shall be
dual licensed as above, without any additional terms or conditions.

The name comes from *surimono* (摺物), privately commissioned Japanese woodblock
prints.
