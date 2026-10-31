# Changelog

All notable changes to surimono are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a minor
release (0.x.0) may change the public API, and every such change is listed here.

The Rust crates and the Ruby gem share one version number.

## [0.1.0] - Unreleased

<!-- The v0.1 release PR (P1-PKG-07) replaces "Unreleased" with the release
     date. Every line below describes code merged on main. -->

The first release: chart islands.

### Added

- **Island markers.** `<script type="application/vnd.surimono+json"
  data-renderer="…" data-width="…" data-height="…">` elements holding a JSON
  spec are replaced with
  `<div class="surimono-island …" id="…" data-renderer="…" style="width:Wpx;height:Hpx;overflow:hidden">`
  holding the SVG: a box of the island's declared size (default 600 × 400 CSS
  pixels) that clips anything drawn outside it. The marker's `id`, `class`,
  `style`, `title`, `lang`, `dir`, `role` and `aria-*` attributes are kept.
  Everything else in the document is left byte for byte as it was. A malformed
  marker (no `data-renderer`, a bad size, JSON that doesn't parse) is an
  `invalid-island` error that names the island.
- **Built-in renderers**, precompiled to QuickJS bytecode at build time and
  checked against the engine and the bundle source before use
  (`SURIMONO_PRECOMPILE=0` compiles from source instead). `builtin_renderers()`
  lists the built-ins in a build and `precompiled_builtins()` says how many of
  them loaded precompiled bytecode, for checking a build or a packaged gem:
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
- **Rust API** in the `surimono` crate: `Renderer::new(Options)`,
  `Renderer::render_islands`, `Renderer::render_islands_cancellable`;
  `Options` (`font`, `limits`, `seed`, `error_policy`, `renderers`, `bundle`,
  `pool_size`, `now_ms`, `utc_offset_minutes`, `text_mode`, `max_islands`);
  `Limits`, `ErrorPolicy`, `TextMode`, `DEFAULT_NOW_MS`;
  `surimono::builtin_renderers()` (the built-in renderer names, sorted) and
  `surimono::precompiled_builtins()` (how many of them loaded precompiled
  bytecode in this process); one
  `#[non_exhaustive]` `Error` enum with `island()` and a stable `kind()`.
  `Renderer` is `Send + Sync`.
- **Fork safety.** A `Renderer` built before a fork (Puma `preload_app!` or
  `fork_worker`, Unicorn, Resque) starts its runtime pool and text measurer
  again in the child on first use, without touching any lock a parent thread
  may have held.
- **Runtime pool** that starts with one runtime and grows to `pool_size`
  (default: the number of CPUs, at most 4; at most 64) only when renders
  overlap; idle runtimes free their memory.
- **Ruby gem** `surimono`: `Surimono::Renderer.new(fonts:, limits:, seed:,
  now:, utc_offset_minutes:, error_policy:, renderers:, bundles:, text:,
  pool_size:, max_islands:)`, `Renderer#render`, and
  `Surimono.render(html, **options)`, which keeps up to 4 renderers;
  `Surimono.builtin_renderers` and `Surimono.precompiled_builtins`. Errors
  are `Surimono::Error` subclasses (`MemoryLimitError`, `TimeLimitError`,
  `StackOverflowError`, `JavaScriptError` with `js_stack`, `RenderError`,
  `ConfigurationError`, `InternalError`) with `kind`, `island_index`,
  `island_id` and `renderer`. Renders release the GVL. CRuby 3.2 to 4.0;
  precompiled gems for x86_64 and aarch64 Linux (glibc) and x86_64 and arm64
  macOS.
- **CLI** `surimono render <in.html> -o <out.html> --font <file>…` (crate
  `surimono-cli`) with the facade's options and typed exit codes: 0 success,
  1 island failed, 2 usage, 3 limit, 4 configuration, 5 I/O. Output is written
  through a temporary file and renamed into place only after success.
- **Licence notices.** Every gem includes `THIRD_PARTY_LICENSES.txt` with the
  licences of all Rust crates and JavaScript libraries built into it.

[0.1.0]: https://github.com/kwallaschek/surimono/releases/tag/v0.1.0
