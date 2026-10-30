# Changelog

All notable changes to surimono are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a minor
release (0.x.0) may change the public API, and every such change is listed here.

The Rust crates and the Ruby gem share one version number.

## [0.1.0] - Unreleased

<!-- Draft. The v0.1 release PR (P1-PKG-07) checks every line against what
     merged, then replaces "Unreleased" with the release date. -->

The first release: chart islands.

### Added

- **Island markers.** `<script type="application/vnd.surimono+json"
  data-renderer="…" data-width="…" data-height="…">` elements holding a JSON
  spec are replaced with
  `<div class="surimono-island …" id="…" data-renderer="…"><svg>…</svg></div>`.
  Everything else in the document is left byte for byte as it was. Islands
  default to 600 × 400 CSS pixels.
- **Built-in renderers** `echarts` (Apache ECharts, server-side SVG) and
  `vega-lite` / `vega`, each a Cargo feature, on by default and included in the
  gem. <!-- confirm: renderer and feature names when P1-RT-06/07 land -->
- **Bring your own bundle.** Any script that defines a global
  `entry(specJson, ctx)` returning an SVG string, or a Promise of one, can be
  registered as a renderer.
- **Sandbox.** Each island runs in a fresh QuickJS context with memory (64 MB),
  time (2 s) and stack (512 KB) limits and no filesystem, network or module
  access. Hitting a limit raises a typed error.
- **Determinism.** Seeded `Math.random`, fixed `Date` in UTC, text measured only
  from the fonts passed in. The same input and options give the same bytes.
- **Text metrics** for `measureText` from the caller's TrueType, OpenType and
  collection files, shaped with Parley 0.6 (the shaper fulgur 0.40 prints with).
- **SVG cleaning** before inlining: scripts, event handlers and external URLs
  removed, ids prefixed per island, font families rewritten to the registered
  fonts.
- **Rust API** in the `surimono` crate: `Renderer::new(Options)`,
  `Renderer::render_islands(&str)`, `Options` builder (fonts, limits, seed,
  error policy, renderers, bundles, pool size), `Limits`, `ErrorPolicy` and one
  `#[non_exhaustive]` `Error` enum.
- **Ruby gem** `surimono`: `Surimono::Renderer.new(fonts:, limits:, seed:,
  error_policy:, renderers:, bundles:, pool_size:)`, `Renderer#render`,
  `Surimono.render(html, **options)`, and `Surimono::Error` with subclasses per
  failure. Renders release the GVL. CRuby 3.2 to 4.0; precompiled gems for
  x86_64 and aarch64 Linux (glibc) and x86_64 and arm64 macOS.
- **Error policy** `:fail` (default) or `:placeholder`, which leaves
  `<div class="surimono-island surimono-island-error" data-error="…">` for a
  failed island and renders the rest.

[0.1.0]: https://github.com/kwallaschek/surimono/releases/tag/v0.1.0
