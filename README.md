# surimono

Run a document's JavaScript without a browser, so browserless HTML-to-PDF
engines can print it.

surimono is a Rust library, a Ruby gem and a command-line tool. It runs the scripts a document needs in
a sandboxed QuickJS runtime and returns HTML that an engine such as
[fulgur](https://github.com/fulgur-rs/fulgur) can print. Version 0.1 does this
for charts: it finds chart *islands* in your HTML, draws them as SVG with
ECharts or Vega, and inlines the SVG.

> **Status:** v0.1 is in development and not yet on crates.io or RubyGems.

## Why

Browserless PDF engines (fulgur, ironpress, rustyprint) start fast and use
little memory, but they don't run scripts. So one chart in an invoice or report
sends you back to Chrome through Grover, Ferrum or a Gotenberg sidecar, with
Chrome's memory use and start-up time. surimono runs only the JavaScript the
document needs, without a browser, and then gets out of the way.

<!-- bench: grover-vs-surimono -->
_Time and peak memory against Grover, cold and warm, will be shown here once the
benchmark in `bench/` has results. Until then this README gives no numbers._

## What v0.1 does

- **Islands.** It replaces each
  `<script type="application/vnd.surimono+json" data-renderer="…">` marker in
  your HTML with the SVG its renderer draws. Everything else in the document
  stays byte for byte as it was.
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
  filesystem or network access.
- **Ruby, Rust and a CLI.** The gem, the crate and the `surimono` command take
  the same options and give the same bytes for the same input.
- **Safe to share and to fork.** One renderer serves many threads, and a
  renderer built before Puma, Unicorn or Resque forks works in every child.

## What it is not

- **Not a browser.** v0.1 runs island renderers only. The page's own `<script>`
  elements pass through untouched and are not run.
- **Not for websites.** It doesn't fetch pages, styles, images or data, and it
  has no network access.
- **Not a PDF engine.** Pair it with fulgur or any engine that prints HTML with
  inline SVG.

## Install

**Ruby** (CRuby 3.2 to 4.0):

```ruby
# Gemfile
gem "surimono"
gem "fulgur"   # or another browserless PDF engine
```

Precompiled gems cover x86_64 and aarch64 Linux (glibc) and x86_64 and arm64
macOS. On other platforms the gem builds from source and needs a Rust
toolchain. fulgur's gem 0.40.0 supports Ruby 3.1 to 3.4, so the pair runs on
Ruby 3.2 to 3.4 today.

**Rust** (1.87 or later; a C compiler builds QuickJS):

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
same branch or `rev` so they come from one commit. On Rust 1.87, put `edition = "2024"` and
`rust-version = "1.87"` in your package so that Cargo picks dependency
versions that build on 1.87; some current releases need 1.88.

ECharts and Vega are Cargo features, both on by default: `echarts` adds the
`echarts` renderer, `vega` adds `vega` and `vega-lite`. To leave Vega out, add
`default-features = false, features = ["echarts"]` to the `surimono` line.
fulgur 0.40 depends on krilla 0.7, which needs Rust 1.89, so a project that
uses both needs 1.89.

**Command line:** build it from a clone of the repository.

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

## Command-line example

```sh
surimono render report.html -o report.out.html \
  --font fonts/NotoSans-Regular.ttf --font fonts/NotoSansJP-Regular.otf
```

`-` reads standard input or writes standard output, and `-o` defaults to
standard output. The output file is written only after the render succeeded.
`surimono --help` lists every option (limits, `--seed`, `--now-ms`,
`--utc-offset-minutes`, `--error-policy`, `--text`, `--bundle`,
`--renderers`, `--max-islands`, `--pool-size`). Exit codes tell a script what
to fix:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | An island failed (bad marker, unknown renderer, script error, unusable SVG), more islands than `--max-islands`, or a document that can't be rewritten |
| 2 | Usage error: an unknown option or a bad value |
| 3 | A script hit its memory, time or stack limit |
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

Every island runs in a fresh QuickJS runtime with these limits:

| Limit | Default | Ruby option | Error when exceeded |
|---|---|---|---|
| JavaScript heap | 64 MB | `limits: { memory_mb: }` | `Surimono::MemoryLimitError` / `Error::MemoryLimit` |
| Running time | 2 s | `limits: { time_ms: }` | `Surimono::TimeLimitError` / `Error::TimeLimit` |
| Stack | 512 KB | `limits: { stack_kb: }` | `Surimono::StackOverflowError` / `Error::StackOverflow` |

A script that hits a limit stops with that typed error. It doesn't crash or
hang your process. Scripts get the JavaScript language and nothing else: no
filesystem, network, module imports, DOM, timers or `console`. The built-in
renderers bring the few stubs their libraries need.

The limits apply to each island, so a document with N islands can take up to
N × the time limit. Two things bound a whole document:

- **`max_islands`** (default 1,000): a document with more islands fails with
  `too-many-islands` before any island runs.
- **Cancellation.** In Ruby, `Timeout.timeout`, `Rack::Timeout`,
  `Thread#raise` or a signal handler that raises in the rendering thread stops
  a render before its next island. In Rust,
  `Renderer::render_islands_cancellable` takes an `AtomicBool` to the same
  effect. An island already running ends within its own time limit.

The SVG that comes back is cleaned before it is inlined: no `<script>`, event
handlers, style sheets, `<foreignObject>`, animation or external URLs, and
every `id` prefixed per island, so no chart can reach the page or another
chart. Its text is then drawn as outlines (see [Fonts](#fonts)).

The same HTML and options give the same bytes on every run and every machine:

- `Math.random` is seeded (the `seed` option, default 0).
- `Date` sees a fixed "now", 2026-01-01T00:00:00Z unless you set `now:`
  (Ruby) or `Options::now_ms` (Rust), and local time is UTC unless you set
  `utc_offset_minutes:` (540 for Japan). The server's clock and time zone are
  never read.
- Text is measured only with the fonts you pass in. System fonts are never read.

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

## Limits of v0.1

- **Page scripts don't run.** Only islands do. Running the page's own scripts on
  a real DOM is planned for v0.2.
- **Scripts can't measure layout.** `getBoundingClientRect` and friends are
  planned for v0.3. Documents that need them still need Chrome for now.
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
  [Sandbox and determinism](#sandbox-and-determinism)).
- **Vega data and size.** Vega and Vega-Lite specs keep their own `width` and
  `height` (a Vega-Lite `"container"` size means the island's). Images, links
  and transforms that draw to a raster canvas (label placement, word clouds,
  heatmap images) aren't supported.
- **Dense charts are slow.** A chart with thousands of marks can take more than
  a second in QuickJS and may need a time limit above the default 2 s.
- **QuickJS is not V8.** Heavy charts run slower than in Chrome. The benchmark
  will show by how much.
- **Mermaid is not included in v0.1.**
- **Platforms.** No Windows or musl gems yet.
- **Rust crates come from the repository.** They aren't on crates.io yet; see
  [Install](#install).

## Compatibility

[docs/COMPAT.md](docs/COMPAT.md) lists what is known to work and what is known
not to, generated from the test results. Every gap is listed there rather than
hidden.

## Roadmap

| Version | Adds |
|---|---|
| 0.1 | Chart islands (this release) |
| 0.2 | The page's own scripts on a real DOM (blitz-dom), on a virtual clock |
| 0.3 | Layout queries answered from the same layout the PDF is printed from; Canvas2D as vectors |

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
