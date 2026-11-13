# Changelog

All notable changes to surimono are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a minor
release (0.x.0) may change the public API, and every such change is listed here.

The Rust crates and the Ruby gem share one version number.

## [Unreleased]

### Added

- **Page scripts.** A document's own scripts run in the sandbox, against a DOM
  with no layout, on a virtual clock until the page settles (#143):
  `Renderer::render_page`, `render_page_cancellable` and `render_page_loading`
  in Rust, returning `PageOutput { html, report }`; `Surimono.render_page`,
  `Renderer#render_page(html, loader:, allow:)` and `render(html, scripts: true)`
  in Ruby, with `Surimono::Page` and `Surimono::PageReport`; `--scripts`,
  `--keep-scripts`, `--no-printer-safe`, `--layout-queries`, `--resource` and
  `--output-mb` on the CLI. `render` and `render_islands` still never run a
  page's scripts.
- **Report or raise.** A page's uncaught exceptions, unhandled rejections and
  files it didn't get go in a report, and the page goes on. Limits are errors:
  `PageMemoryLimit`, `PageTimeLimit`, `PageStackOverflow`, `PageDomLimit`,
  `PageLayoutQuery` and `PageFailed` in Rust; `DomLimitError`, `LayoutQueryError`
  and `RenderError` with kind `"page"`, along with the existing limit errors, in
  Ruby.
- **`Limits::output_bytes`** (`limits[:output_mb]`, default 64 MiB) caps a page's
  output and the text it may hold.
- **No files by default.** A page loads only through a `ResourceLoader` (Ruby:
  `resources:`, or `loader:` with `allow:`), which sees URLs as written. A
  stylesheet it loads comes out as `<style>`; a `url()` or `@import` in it isn't
  followed.
- **Layout queries answer zeros, or fail the render** (`LayoutQueries`,
  `layout_queries:`) until layout lands in phase 3.
- **A page's output is serialized**, so it isn't byte-identical to the input:
  `<script>` (unless `keep_scripts`), `<noscript>` and comments are gone, it
  is screened so the printer's parse matches ours, and it is printer-safe
  (below).
- **Printer-safe output**, on by default (#180): `Options::printer_safe(bool)`
  in Rust, `Surimono::Renderer.new(printer_safe:)` in Ruby, `--no-printer-safe`
  (with `--scripts`) on the CLI. A page's output is written in forms stock
  fulgur 0.40 prints the way Chrome prints the page: a block `<svg>` keeps the
  size its `width` and `height` give it and a border on one side prints in its
  own colour, through one `<style>` added in `<head>` that ranks below all of
  the page's CSS, and a newline ending a `<pre>` is left out. Each is made
  only where Chrome prints the result exactly as the page. Where the page
  can't show that, it is left as it is: among others, with a script that may
  be left in it in any form (`keep_scripts`, any attribute whose name starts
  with `on`, `srcdoc`, a `javascript:` URL, a frame that loads a document),
  CSS that isn't in the page or that the pass can't read to its end, an HTML
  element it doesn't know (`<select>`), a `<style>` with a child that isn't
  text, or a declarative shadow root. A page in quirks mode gets only the
  `<svg>` sizes. Out of steps or time, the plain page goes out: the pass
  never fails a render. With it, 4 of the 6 real-world pages pass on stock
  fulgur 0.40.0, against 2 without. `printer_safe(false)` gives the
  serialized DOM as before.
- **Platform globals for page scripts** (#160): `URL` (input and output capped
  at 2 MiB, past which it throws a `TypeError`), `URLSearchParams`,
  `TextEncoder`, `TextDecoder` (every encoding of the Encoding Standard),
  `atob`, `btoa` and `structuredClone`. `navigator` is fixed values: user
  agent `Mozilla/5.0 (compatible; surimono)`, `en-US`, an empty `platform`,
  `onLine` and `cookieEnabled` false, `hardwareConcurrency` 1.
  `customElements` is a registry only: a defined element is never constructed
  or upgraded. `Intl` isn't there: it isn't in the engine.
- **HTML members templates use** (#161): `innerText`; `focus`, `blur`,
  `tabIndex`, `document.activeElement` and `hasFocus`; tables (`rows`,
  `cells`, `insertRow`, `deleteRow`, `insertCell` and the rest); selects and
  options (`options`, `selectedIndex`, `value`, `new Option`); a form's
  `elements`; `textarea.value`; URL parts on `a` and `area`;
  `document.forms`, `links`, `images` and `anchors`; `new Image`. An option's
  `selected` is its attribute, as `checked` is, so what a script chooses is
  printed. `innerText` sees no stylesheet until phase 3, `document.cookie` is
  empty and a write does nothing, and `:focus` doesn't match.
- **Tree traversal** (#162): `document.createTreeWalker`,
  `document.createNodeIterator` and `NodeFilter`. Each `NodeIterator` counts
  4 KiB against the text a page may hold, until the page ends.
- **Style sheets from scripts** (#163): `document.styleSheets`, a style
  element's `sheet`, `cssRules`, `insertRule`, `deleteRule`, `addRule` and
  `removeRule`. Rules a script inserts are printed, so CSS-in-JS styles reach
  the PDF. `insertRule` checks a rule's structure, not its selectors or
  declarations. Minified sheets work too: a selector that ends in a bracket
  right before `{` (`.a:not(.b){…}`) keeps its whole `selectorText`, and the
  rules after it still print once a script edits the sheet (#181).
- **Docs.** Page scripts in the Rails and Rust guides and the README, a page
  example in Rust and Ruby, and the [checkpoint 3 checklist](docs/guide/checkpoint-3.md).
- **musl native gems, prepared** (#170): the gem build makes `x86_64-linux-musl`
  and `aarch64-linux-musl` gems and tests the x86_64 one on Alpine Linux, Ruby
  3.2 to 4.0. The nightly build runs them once its workflow change is adopted;
  none has been built on CI yet, and none is released.

### Changed

- **`Math` gives the same bits on every platform** (#171). `Math.sin`, `cos`,
  `tan`, `asin`, `acos`, `atan`, `atan2`, the hyperbolic functions and their
  inverses, `exp`, `expm1`, `log`, `log1p`, `log2`, `log10`, `cbrt`, `pow` (and
  `**`) and `hypot` are correctly rounded, from CORE-MATH (MIT), instead of the
  platform's C library. Some results change in the last bit; the chart and
  real-world fixtures give the same bytes as before on x86_64 Linux.

- **Rust 1.89 is the minimum supported version**, up from 1.87 (#117). fulgur
  0.40 needs it too.
- **A cancel stops the running island**, not only the islands after it (#108):
  `Renderer::render_islands_cancellable` in Rust; in Ruby, `Timeout`,
  `Thread#raise`, `Thread#kill` and signals. Loading a bundle still isn't
  interrupted, so a cancel during a built-in renderer's compile from source,
  or a bundle's compile at a forked process's first render, takes effect when
  the load ends.

### Fixed

- **Ruby 3.2:** a signal now reaches a render on the main thread while other
  Ruby threads exist, instead of waiting for the render to end (#114). The gem
  runs a sleeping helper thread, `surimono-signal-pump`, for the length of such
  a render. Ruby 3.3 and later need none.
- **The engine's time limits stop compiles, `JSON.parse` and regexp compiles at
  the limit** (#125). They used to run on past it: a small JSON spec could
  start a regexp compile that nothing interrupted. A compile from source now
  runs under the render's time limit, so loading a bundle can fail with a time
  limit error when the limit is short.
- **Proxy `ownKeys` checks** (#126). The check of a proxy's `ownKeys` result is
  no longer quadratic, and `Object.keys`, `Object.entries` and
  `Object.getOwnPropertyDescriptors` now stop at the time limit.
- **A time limit is always a time-limit error** (#178). Rarely, when the
  watchdog was slow to stop a script, the limit came out as an ordinary
  `InternalError: interrupted` exception, and the script's state was kept.
  It now always ends with the time-limit error, and the state is rebuilt
  from scratch, as after any limit.

## [0.1.0] - 2026-11-04 (not released)

> **Not released.** The `v0.1.0` tag exists, on commit `915d5cbf`, but no gem
> was pushed to RubyGems, so the install lines below don't work. That commit
> has a known gap: the time limit doesn't stop the regexp compiler, which is
> fixed on `main` (#125). The first release is planned for v0.3. This entry is
> kept as written for the planned release.

The first release: chart islands.

### Distribution

- **Ruby gem** `surimono` on RubyGems: `gem install surimono`, or
  `gem "surimono"` in a Gemfile.
- **CLI** built from a clone of the repository:
  `cargo install --locked --path crates/cli`.
- **The Rust crates are not on crates.io in v0.1**: they need a patched
  QuickJS engine, which a published crate can't carry yet (Cargo's `[patch]`
  doesn't travel with it). Rust projects can depend on the repository with
  git, plus a `[patch.crates-io]` entry for `rquickjs-sys` in their own
  `Cargo.toml`, as the [Rust guide](docs/guide/rust.md#add-the-crates) shows.

### Added

- **Island markers.** `<script type="application/vnd.surimono+json"
  data-renderer="…" data-width="…" data-height="…">` elements holding a JSON
  spec are replaced with
  `<div class="surimono-island …" id="…" data-renderer="…" style="width:Wpx;height:Hpx;overflow:hidden">`
  holding the SVG: a box of the island's declared size (default 600 × 400 CSS
  pixels) that clips anything drawn outside it. The marker's `id`, `class`,
  `style`, `title`, `lang`, `dir`, `role` and `aria-*` attributes are kept;
  its `style` comes after the box's size. Everything else in the document is
  left byte for byte as it was. A malformed marker (no `data-renderer`, a bad
  size, JSON that doesn't parse) is an `invalid-island` error that names the
  island.
- **Built-in renderers**, precompiled to QuickJS bytecode at build time and
  checked against the engine and the bundle source before use. Bytecode that
  doesn't match is compiled from source instead, with the same output, as
  with `SURIMONO_PRECOMPILE=0`. `builtin_renderers()` lists the built-ins in
  a build and `precompiled_builtins()` says how many of them loaded
  precompiled bytecode, for checking a build or a packaged gem:
  - `echarts`: Apache ECharts 6.1.0, server-side SVG, at the island's size,
    animation off (Cargo feature `echarts`);
  - `vega` and `vega-lite`: Vega 6.4.0 and Vega-Lite 6.5.0, headless view to
    SVG, inline data only (Cargo feature `vega`).

  Both features are on by default and both are in the gem.
- **Bring your own bundle.** A classic script defining a global
  `entry(specJson, ctx)` that returns an SVG string, or a Promise of one, is a
  renderer. `ctx` has `width`, `height` and `measureText(text, cssFont)`. It is
  compiled once and runs under the same limits as the built-ins.
- **Sandbox.** Each island runs in a fresh QuickJS runtime with memory (64 MiB),
  time (2 s, covering loading and drawing) and stack (512 KiB) limits and no
  filesystem, network or module access. Hitting a limit is a typed error,
  never a crash or a hang.
- **Document bounds.** `max_islands` (default 1,000) fails a document with more
  islands before any of them runs (`too-many-islands`). A render can be
  cancelled between islands: `Renderer::render_islands_cancellable(html, &AtomicBool)`
  in Rust; in Ruby, `Timeout`, `Thread#raise`, `Thread#kill` and signals stop
  a render before its next island.
- **Determinism.** Seeded `Math.random` (`seed`), a fixed clock
  (`now` / `Options::now_ms`, default 2026-01-01T00:00:00Z) and time zone
  (`utc_offset_minutes`, default UTC), and text measured only with the fonts
  passed in. The same input and options give the same bytes.
- **Text metrics** for `measureText` from the caller's TrueType, OpenType and
  collection files, shaped with Parley 0.6 (the shaper fulgur 0.40 prints
  with). Generic families map to the registered fonts; other registered fonts
  are a per-character fallback in the order given. WOFF and WOFF2 are rejected.
- **SVG sanitising** before inlining: scripts, event handlers, style sheets,
  `<foreignObject>`, SMIL animation and external URLs removed, ids prefixed per
  island, font families rewritten to the registered fonts. Output that can't
  be made safe is a `render` error.
- **Chart text as outlines** by default: island text is drawn as vector paths
  from the registered fonts, shaped exactly as it was measured, so it prints
  the same whatever fonts the PDF engine has (fulgur 0.40 prints SVG text with
  system fonts). Each label becomes `<g role="img" aria-label="…">`; it can't be
  selected or searched in the PDF. `text: :keep` (Ruby), `TextMode::Keep`
  (Rust) or `--text keep` (CLI) keeps `<text>` instead.
- **Error policy** `fail` (default) or `placeholder`, which leaves an empty
  `<div class="surimono-island surimono-island-error" data-error="<kind>">` of
  the island's size, with the message in a comment, and renders the rest.
- **Rust API** in the `surimono` crate (from the repository; see
  Distribution): `Renderer::new(Options)`, `Renderer::render_islands`,
  `Renderer::render_islands_cancellable`, `Renderer::renderers`;
  `Options` (`font`, `limits`, `seed`, `error_policy`, `renderers`, `bundle`,
  `pool_size`, `now_ms`, `utc_offset_minutes`, `text_mode`, `max_islands`);
  `Limits`, `ErrorPolicy`, `TextMode`, `DEFAULT_NOW_MS`, `VERSION`;
  `surimono::builtin_renderers()` (the built-in renderer names, sorted) and
  `surimono::precompiled_builtins()` (how many of them loaded precompiled
  bytecode in this process); one `#[non_exhaustive]` `Error` enum with
  `island()` (the failed island as an `IslandRef`, when there is one) and a
  stable `kind()`. `Renderer` is `Send + Sync`.
- **Fork safety.** A `Renderer` built before a fork (Puma `preload_app!` or
  `fork_worker`, Unicorn, Resque) starts its runtime pool and text measurer
  again in the child on first use, without touching any lock a parent thread
  may have held.
- **Runtime pool** that starts with one runtime and grows to `pool_size`
  (default: the number of CPUs, at most 4; at most 64) only when renders
  overlap; a runtime idle for 2 s frees its memory, and on Linux (glibc) the
  freed memory goes back to the system.
- **Ruby gem** `surimono`: `Surimono::Renderer.new(fonts:, limits:, seed:,
  now:, utc_offset_minutes:, error_policy:, renderers:, bundles:, text:,
  pool_size:, max_islands:)`, `Renderer#render`, `Renderer#renderers`, and
  `Surimono.render(html, **options)`, which keeps up to 4 renderers;
  `Surimono.builtin_renderers` and `Surimono.precompiled_builtins`. Errors
  are `Surimono::Error` subclasses (`MemoryLimitError`, `TimeLimitError`,
  `StackOverflowError`, `JavaScriptError` with `js_stack`, `RenderError`,
  `ConfigurationError`, `InternalError`) with `kind`, `island_index`,
  `island_id` and `renderer`. Renders release the GVL. CRuby 3.2 to 4.0;
  precompiled gems for x86_64 and aarch64 Linux (glibc) and x86_64 and arm64
  macOS.
- **CLI** `surimono render <in.html> -o <out.html> --font <file>…` (crate
  `surimono-cli`, built from a clone) with the facade's options and typed
  exit codes: 0 success, 1 island failed, 2 usage, 3 limit, 4 configuration,
  5 I/O. Output is written through a temporary file and renamed into place
  only after success.
- **Licence notices.** Every gem includes `THIRD_PARTY_LICENSES.txt` with the
  licences of all Rust crates and JavaScript libraries built into it.

[Unreleased]: https://github.com/kwallaschek/surimono/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kwallaschek/surimono/releases/tag/v0.1.0
