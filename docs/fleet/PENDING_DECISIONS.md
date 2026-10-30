# Pending DECISIONS lines (moved into docs/DECISIONS.md when each PR merges)

## RT batch 1 (#5 #8 #11 #13 #15)
- 2026-10-31 · Sandbox local time is UTC plus a fixed utc_offset_minutes (default 0), never the host's zone · Output must not depend on where the server runs.
- 2026-10-31 · Default Math.random seed 0x73757269_6d6f6e6f and clock 2026-01-01T00:00:00Z; every context restarts the sequence · Same input, same output across pooled workers.
- 2026-10-31 · WeakRef and FinalizationRegistry are strong/no-op shims; performance.now() returns 0 in phase 1 · GC timing must not leak; the phase-2 virtual clock will drive performance.now().
- 2026-10-31 · Island entry is `entry(specJson, ctx)` with the spec as JSON text, validated in Rust first; it may return SVG or a Promise of SVG that settles without timers · BadJson with line and column before any JS runs; Vega's toSVG is async.
- 2026-10-31 · IslandRunner is a fixed pool of worker threads, each owning its sandbox, behind a job queue; a fresh QuickJS runtime per render, prepared after replying · Send + Sync without rquickjs `parallel`; rquickjs 0.14 caches class prototypes per runtime (pins the first context), and a fresh runtime costs the same as a fresh context.
- 2026-10-31 · TextMeasurer::approximate() is the islands default until gfx is wired in · Deterministic and font-free; the facade always passes the gfx measurer.

## PKG batch 1 (#4 #7 #10 #14)
- 2026-10-31 · Ruby: Surimono::Renderer.new(fonts:, limits: {memory_mb:, time_ms:, stack_kb:}, seed:, error_policy:, renderers:, bundles:, pool_size:), #render(html), Surimono.render(html, **opts) caching ≤16 renderers · Mirrors the Rust facade in Ruby units.
- 2026-10-31 · Ruby errors: Surimono::Error < StandardError with MemoryLimitError, TimeLimitError, StackOverflowError, JavaScriptError, RenderError, ConfigurationError · ::ScriptError exists and isn't a StandardError.
- 2026-10-31 · In fonts:, a binary-encoded String is font data, any other String or Pathname is a path · File.binread returns binary Strings.
- 2026-10-31 · Renders run under rb_thread_call_without_gvl with no unblock function · Other threads run; interrupts wait and the time limit bounds the wait.

## GFX batch 1 (#3 #12 #16)
- 2026-10-31 · surimono-gfx measures with parley =0.6.0 and skrifa =0.37.0 (no system feature) · Same shaper fulgur 0.40 prints with.
- 2026-10-31 · A family list resolves to registered families it names (or the sans-serif mapping), then every other registered family as fallback · Browser-like CJK fallback without system fonts.
- 2026-10-31 · TextMetrics rounds font ascent/descent and snaps ink boxes to whole px like Chrome on Linux; serializes with canvas camelCase names · Matches Chrome to 0.0007 px.
- 2026-10-31 · WOFF/WOFF2 rejected for now; measureText rejects text over 256 KiB; ex/ch = 0.5em, viewport units invalid in canvas fonts · No brotli dep; shaping can't be interrupted; no viewport at parse time.
- 2026-10-31 · sanitize_svg(svg, island, &FontSet) with MAX_SVG_NODES 1,000,000 and MAX_SVG_DEPTH 128, roxmltree =0.21.1, edits by byte range; ids prefixed sm{island}-; only #fragment and data:image/ refs kept (plus http/https/mailto on <a>); CDATA escaped, comments/PI/DOCTYPE dropped, HTML breakout elements removed; DTD or deep/malformed SVG rejected · One place for safety, ids and fonts; untouched bytes identical.
- 2026-10-31 · Callers give fulgur the same font files with system fonts off · Proven exact by tests/fulgur_layout.rs; generic families print blank without system fonts, so page CSS must name registered families.

## Upstream drafts to write (orchestrator)
- fulgur-ruby: expose system_fonts: and AssetBundle#add_font_bytes
- fulgur: map generic families to bundled fonts
- (RT will write quickjs-ng interrupt + rquickjs prototype cache drafts in P1-RT-09)

## CF batch 1 (#1 #2 #6 #9 #17)
- 2026-10-31 · The real-world suite pins each library by npm tarball, sha512 integrity and sha256 of the exact bundle file · One manifest that the harness and crates/islands are checked against.
- 2026-10-31 · Script results must be strings or real Promises; the host never calls toString/then on a result; a promise that can't settle ends at once · Rust never re-enters hostile JS outside the limits.
THIRD_PARTY rows:
| Noto Sans (test fixture font) | 2.015 | OFL-1.1 | https://github.com/notofonts/notofonts.github.io (412a7edb2c8c535d1066157e5b17e661fd5591fe) | fixtures/fonts/ |
| Noto Sans JP (test fixture font) | 2.004 | OFL-1.1 | https://github.com/notofonts/noto-cjk (f8d157532fbfaeda587e826d4cd5b21a49186f7c) | fixtures/fonts/ |
TODO: upstream drafts for 5 fulgur 0.40 gaps from COMPAT.md (#17); RT task: Rust hostile harness over fixtures/hostile/manifest.toml.

## Review outcomes / extra decisions
- Islands review: #11 APPROVE; #13 BLOCK (eprintln probe); #15 BLOCK (soak metric) -> sent to RT batch 2 agent 02:3x.
- Decision: soak primary check = live heap bytes via counting global allocator (dev-dep stats_alloc if deny passes; else one test file opt-in unsafe); RSS printed with derived bound.
- Decision: library crates add #![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))] (owners).
- Add DECISIONS for public island API (IslandRunner, RunnerOptions, IslandRequest, IslandError, TextMeasurer, TextMetrics), Sandbox::run_await, IslandRunner::stats()/RunnerStats.

## 02:45 review round 2
- PKG review: #4 #7 #10 APPROVE; #14 BLOCK (longjmp over Rust frames on Thread#raise) -> PKG batch 2 agent.
- CF review: #1 #2 #6 APPROVE; #9 BLOCK (json-stringify expectation), #17 BLOCK (page-height rounding) -> CF batch 2 agent. Metric colour-blind follow-up sent.
- GFX review: #3 BLOCK (cache byte budget), #12 BLOCK (style in integration points), #16 BLOCK (fulgur prints inline SVG text with SYSTEM fonts) -> GFX batch 2 agent.
- DECISION 2026-10-31 · Island text is converted to vector outlines in the registered fonts by default (TextMode::Outlines; Keep is opt-in) · fulgur 0.40 prints inline SVG via blitz-dom's usvg with a system-font database, so <text> would print in host fonts; a blitz [patch] can't help users who install the fulgur gem. Cost: chart labels not selectable in PDF (aria-label kept). Flag to Kay at checkpoint 2.
- DECISION · sanitize_svg drops @font-face rules · src: local() reaches system fonts.
- MERGE BLOCKED: auto-mode classifier refused PR merge ("Merge Without Review") at 02:40; asked Kay for explicit go.
- TODO THIRD_PARTY: fonts rows; robust-predicates (Unlicense) inside vega.min.js.
- TODO docs/upstream: blitz-svg-document-fonts (GFX writes), 5 fulgur gaps (from COMPAT in #17).
- Core review: #5 APPROVE (merge; v0.1 release blockers R1 sticky interrupt + native loop polls, R2 watchdog tests, R3 docs); #8 BLOCK (B1 prepareStackTrace leak of native Date, B2 new Date(obj) host zone, B3 Date.parse zone abbreviations). New agent "RT sandbox hardening" owns crates/core + vendor/rquickjs-sys + [patch] + docs/upstream quickjs/rquickjs drafts. RT renderer agent keeps crates/islands.
- RULE 2026-10-31 · Never rebase or force-push a pushed branch; fix with new commits; downstream merges parent in · Force-push may be refused; merge commits keep stacks valid.
- COMPAT: toLocale*String ignores locale ('ja-JP').

## DX batch 1 (#18 #19 #23), add on merge
- 2026-10-31 · Docs quote no speed or memory figures until bench/ has results · Every performance claim must cite a benchmark.
- 2026-10-31 · Ruby examples write island JSON with `raw json_escape(spec.to_json)` · Plain ERB escapes quotes and breaks the JSON.
- 2026-10-31 · The Rails guide builds renderers lazily per process (keyed by Process.pid) until the facade is fork-safe · Runner threads don't survive fork.
- 2026-10-31 · Jobs discard Surimono::Error but retry TimeLimitError · Only time limits depend on machine load.
- 2026-10-31 · Examples are a workspace crate plus a plain-Ruby snippet, driven by examples/run.sh · Examples that run in CI can't rot.
- 2026-10-31 · Examples use ECharts plus a bring-your-own bundle, not Vega · Vega is second in the cut order.
- ORC TODO after #10 merges: add examples/rust-islands to workspace members/default-members.

## Orchestrator
