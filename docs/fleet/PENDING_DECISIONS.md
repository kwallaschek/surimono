# Pending DECISIONS lines (moved into docs/DECISIONS.md when each PR merges)

## RT batch 1 (#5 #8 #11 #13 #15)
- (superseded by #30: islands uses surimono_gfx::TextMeasurer; no fonts → IslandError::NoFonts; the estimate is test-only)

## PKG batch 1 (#4 #7 #10 #14)

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

## CF batch 2 (#2 #9 #17 #24 #28 #31), add on merge
- (compat.py line moved to DECISIONS on #28 merge)

## Open for checkpoint 2 (Kay)
- First real benchmark (CI, before #25 merged, branch release/ci-check-p1-cf-06-pkg08): surimono+fulgur Ruby cold 1,352 ms / warm 686 ms vs Grover 2,153 / 1,581 → 1.6× cold (not met), 2.3× warm (met); PSS 154/179 MB vs 582/704 → 3.8×/3.9× (10× not met; fulgur alone is only 9.4×/8.6×). Grover on chrome-headless-shell is a second baseline (376/496 MB). P1-RT-16 (memory) and P1-RT-17 (precompiled bytecode for cold start) are trying to close the gap.
- 10× memory target: fulgur alone already ~85 MB PSS vs Grover 580 MB (6.8×); on summed RSS 13.7×. Which measure does SPEC mean? Report honestly.
- MSRV: fulgur 0.40 needs rustc 1.89 (krilla 0.7); Rust users pairing surimono with the fulgur crate can't use 1.87 anyway. Plan: raise MSRV to 1.89 at the start of phase 2 (one planned task, full rebuild), not mid-checkpoint.
- Vega speed: scatter 2,000 pts ~1.2 s under load vs 2 s default limit.

## RT sandbox hardening (#8 fixes, #32), add on merge
- 2026-10-31 · QuickJS (quickjs-ng 0.16.2 via rquickjs-sys 0.14.0) is vendored in vendor/rquickjs-sys with [patch.crates-io]: sticky interrupt, polls in generic element access and sorts, two compiler OOM crash fixes, link marker · The only way to make "never a hang or crash" true; drop when upstream ships the fixes.
- 2026-10-31 · The prelude's array-like length guards are removed · The patched engine covers those loops and the guards had holes.
- 2026-10-31 · surimono-core links only against the patched engine (marker symbol) · An unpatched build fails loudly instead of shipping a sandbox that can hang.
- 2026-10-31 · The sandbox returns InvalidOptions on a thread with less than stack_bytes + 256 KiB of stack left · Deep recursion on a small thread aborted the process.
- 2026-10-31 · The OOM sweep covers allocations a script can cause, not runtime or context creation · Creation needs ~200 KiB, under the 1 MiB minimum limit.
- 2026-10-31 · New public API: Sandbox::STACK_HEADROOM_BYTES, Limits::{MAX_MEMORY_BYTES, new, with_*}, SandboxOptions::with_* · Builders for non_exhaustive structs.
THIRD_PARTY row on #32 merge:
| QuickJS-ng (patched, via rquickjs-sys) | 0.16.2 (rquickjs-sys 0.14.0) | MIT | https://github.com/quickjs-ng/quickjs, https://crates.io/crates/rquickjs-sys | vendor/rquickjs-sys/ (changes in PATCHES.md) |
Open for Kay (release): [patch] doesn't reach crates.io users; publishing surimono-core needs the forked engine published under its own crate names until upstream ships the fixes (the gem is unaffected). Upstream drafts in docs/upstream/ for Kay to file.
Known gap: allocation failure during runtime/context creation (real system OOM only) can still crash; not reachable through the memory limit.

## GFX batch 2 (#3 #12 #27 #16) — replaces the GFX batch 1 lines above on merge
- 2026-10-31 · surimono-gfx public API: FontSet (new, add_font_bytes, families, face_count, set_generic_family, font_files, resolve_families), TextMeasurer (new, measure, fonts; Clone + Send + Sync), TextMetrics, GenericFamily, GfxError (#[non_exhaustive]: InvalidFont, UnsupportedFontFormat, UnknownFamily, NoFonts, TextTooLong, InvalidSvg), MAX_TEXT_BYTES = 256 KiB · canvas measureText from the caller's fonts with Parley =0.6.0, matching Chrome to 0.0007 px.
- 2026-10-31 · measureText caches are bounded by bytes: keys ≤ 256 B, ≤ 256 KiB per workspace; a css_font over 1 KiB means the canvas default · Hostile input grew the host by 67 to 101 MB in review.
- 2026-10-31 · sanitize_svg(svg, island, &TextMeasurer, &SvgOptions), MAX_SVG_NODES 1,000,000, MAX_SVG_DEPTH 128 · One safe inlining step; the measurer carries fonts and shaping.
- 2026-10-31 · Island <style> is written as plain decoded text, never with entities; dropped if it has child elements, needs escaping or ends open; @font-face dropped; namespaces follow the HTML parser · Review bypass of #12; local() reaches system fonts.
- 2026-10-31 · The island prefix covers ids, ARIA and other id lists, SMIL begin/end and ECharts zrN- classes · Islands must not reference or style each other.
- 2026-10-31 · SvgOptions { text: TextMode::Outlines | Keep }, default Outlines; <text> becomes <g role=img aria-label> shaped with the same Parley path as measure(), glyphs from skrifa, two fixed decimals · fulgur 0.40 prints inline SVG text with system fonts; outline advances are bit-identical to measure() and within 0.049 px of Chrome.
- 2026-10-31 · MAX_OUTLINE_CHARS 20,000 and 32 MiB of path data per SVG → InvalidSvg; outlining with no fonts → NoFonts · Bounds host memory and time.
- 2026-10-31 · tests/fulgur_print.rs runs only where fulgur CLI and poppler exist; SURIMONO_REQUIRE_FULGUR=1 enforces it in a CI job · Keeps PR CI green without fulgur.

## 11:40 · #25 review outcomes
- DECISION 2026-10-31 · Island output can't affect anything outside its own box: sanitize_svg drops every <style>, foreignObject and HTML-namespace content and SMIL; desc/title become text-only; the root <svg> style is allowlisted · An ECharts emphasis colour injected page-wide CSS (body{display:none}); custom bundles could swallow the rest of the document or set page <body> attributes.
- DECISION 2026-10-31 · A forked child rebuilds its FontSet/TextMeasurer from font bytes along with the pool · A parent thread can hold the measurer's lock at fork time (18 of 200 children hung).
- DECISION 2026-10-31 · Optional max_islands per document (default 1,000) and a cancel path: the Ruby unblock function sets a flag the facade checks between islands · Per-island deadlines don't bound a whole document; Timeout/SIGTERM waited for all islands.
- Open: macos-14 runner image retired 2 Nov; full.yml needs macos-15 (YAML for Kay at checkpoint 2).

## Islands #39 (merged; lines moved)

## RT-15 (#45), add on merge
- 2026-10-31 · The OOM sweep switches QuickJS's small-block arena off per runtime through a test-only engine hook, in the normal test suite · With the arena on the allocator sees ~2% of allocations; with it off every allocation can be refused.
- 2026-10-31 · QuickJS assertions stay on, but JS_FreeRuntime leaves leaked objects instead of aborting; the sweep requires every runtime to give back all bytes · A leak costs memory, not safety; disable-assertions would hide other invariant violations.
- 2026-10-31 · Atoms referenced from a failed bytecode buffer are released at runtime teardown instead of by decoding the buffer · A failed buffer can hold partial instructions; the sandbox replaces the runtime after MemoryLimit.
- 2026-10-31 · new Date(y, m, …) follows ECMA-262 §21.4.2.1 (TimeClip after converting to UTC); Date.parse keeps QuickJS's order for zones written in the string · Spec; the 30,000-string differential still matches.
- 2026-10-31 · On musl's main thread the stack check uses RLIMIT_STACK · musl reports only the mapped stack.
- 2026-10-31 · The sandbox requires engine patch level 2 · It must not run without the allocation-failure fixes.
Private: the maintainer report draft is in the Claude project (claude/private-quickjs-ng-security-report.md); Kay sends it.

## PKG batch 3 (#25 #26 #34 #33 #44), add on merge
- 2026-10-31 · A forked child finds its state through pid-claimed slots (atomic loads, CAS) and builds its own TextMeasurer over a lock-free FontSet clone along with its pool; nothing a parent thread may hold is touched · 18 of 200 children hung on the measurer's lock.
- 2026-10-31 · Options::max_islands (default 1,000) → Error::TooManyIslands before any island runs; Renderer::render_islands_cancellable(&AtomicBool) → Error::Cancelled between islands; Ruby max_islands: · Per-island deadlines don't bound a document.
- 2026-10-31 · Ruby renders with an unblock function that sets the cancel flag (not RB_NOGVL_UBF_ASYNC_SAFE); a wake-up with no exception re-renders · Timeout/SIGTERM stop between islands; the async-safe path calls the unblock function without Ruby's lock.
- 2026-10-31 · The CLI takes the facade's options (--bundle name[:fn]=file, --renderers, --max-islands, --pool-size) and writes output via temp file + rename · Gem and CLI share one API; no truncated output files.
- 2026-10-31 · THIRD_PARTY_LICENSES.txt generation fails on an MPL-2.0-only crate not unmodified from crates.io or not named in about.toml/deny.toml, and on a non-licence, non-JS file in a bundle directory; cargo-about runs --frozen --fail · Notices must be complete and true.
- 2026-10-31 · CI logs are published by a separate publish job, the only one with contents: write, running the default branch's script on artifacts (YAML for Kay) · Build steps never see the write token.
