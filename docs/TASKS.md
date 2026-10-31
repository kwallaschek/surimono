# Tasks

One line per task: id · owner · status · task. Status is one of todo, doing,
review, done, cut. Branch name = task id in lower case. Phase 1 is planned in
full; phases 2 and 3 are epics that get broken down when their phase starts.

Owners: ORC orchestrator, CF conformance researcher, RT runtime engineer, DOM DOM
engineer, GFX graphics engineer, PKG packaging engineer, DX docs and DX, REV reviewer.

## Phase 0: bootstrap (Sat 31 Oct)

- P0-01 · ORC · done · Cargo workspace, toolchain pin, exact rquickjs and Blitz pins, deny.toml, licences, .gitignore, CLAUDE.md
- P0-02 · ORC · done · CI workflows ci.yml, full.yml, reference.yml, bench.yml (first cut; PKG owns them from here)
- P0-03 · ORC · done · docs: SETUP, TASKS, STATUS, DECISIONS, COMPAT, SPEC, upstream/
- P0-04 · ORC · done · Checkpoint 1: Kay approved SPEC v1 (31 Oct 01:19)
- P0-05 · ORC · done · GitHub repo attached, history pushed, workflows committed by Kay, CI publishing to ci-results

## Phase 1: islands (v0.1, checkpoint 2 on Sun 1 Nov ~18:00)

Never cut: sandbox limits, determinism, soak test, text measurement, the Ruby gem.
Cut order if late: Mermaid, then Vega, then the CLI.

### Runtime engineer (crates/core, crates/islands)
- P1-RT-01 · RT · done · QuickJS sandbox: one Runtime/Context wrapper with memory, time (interrupt handler) and stack limits; no modules, filesystem or network; typed `SandboxError` (MemoryLimit, TimeLimit, StackOverflow, Exception with message and stack); tests for each limit
- P1-RT-02 · RT · done · Determinism in the sandbox: seeded Math.random (per render seed), fixed Date.now/new Date(), no performance.now drift; test that two runs are byte-identical
- P1-RT-03 · RT · done · Island runner: load a JS bundle once, compile to bytecode, call its entry with JSON, return an SVG string; bring-your-own-bundle API (any `(json) => svg` function)
- P1-RT-04 · RT · done · Runtime pool: reuse runtimes and compiled bundles across renders, reset per render, bounded size; no state leaks between renders (test)
- P1-RT-05 · RT · done · Soak test: 10,000 island renders in one process, live heap flat after warm-up (< 64 KiB growth), resident memory reported with a derived bound (ignored test, run in full.yml and before releases)
- P1-RT-06 · RT · done · ECharts renderer (feature `echarts`): pinned bundle, SSR SVG mode, `setPlatformAPI({ measureText })` wired to gfx text metrics, licence file kept
- P1-RT-07 · RT · done · Vega and Vega-Lite renderer (feature `vega`): headless View to SVG, text measurement wired to gfx, licence files kept
- P1-RT-08 · RT · cut · Mermaid renderer: cut from v0.1 (first in the cut order; its bundle carries elkjs under EPL-2.0)

- P1-RT-09 · RT · done (#32, landed via #39) · QuickJS patch (vendored rquickjs-sys, [patch]): sticky interrupt, interrupt polls in native array loops and typed-array sort; watchdog tests; upstream drafts (v0.1 release blocker)
- P1-RT-10 · RT · done (#30) · Use surimono-gfx's TextMeasurer/TextMetrics in islands (one type each across the API)
- P1-RT-12 · RT · done (#39) · Islands follow-ups from #21/#29 review: Vega image/href refused synchronously with tests; preload time vs deadline; built-in vs registered name race; entry check consistency; idle-wait in tests; docs
- P1-RT-14 · RT · review · (#47) Deadline enforcement: watchdog flag checked on every poll; expensive natives poll (indexOf, BigInt cap); flaky core tests; F2/F3 (v0.1 release blockers R4, R5)
- P1-RT-15 · RT · done (#45) · (#45) Allocation-failure crash fixes in the vendored engine; full-coverage OOM sweep; docs stop over-claiming (v0.1 release blockers R6, R7, R8; details private)
- P1-RT-15b · RT · done (#45) · #45 review blockers: sweep covers bind and Promise.any; nits
- P1-RT-20 · RT · todo · quickjs-ng public issue drafts in docs/upstream/quickjs-ng/, one per finding, each with a reproducer on current master and a repro.sh Kay runs to verify (per quickjs-ng's security policy)
- P1-RT-19 · RT · done (#52) · macOS: the determinism test's own TZ probe read 1970, where macOS libc ignores POSIX-rule transitions; probe 2001 and run IANA zones
- P1-RT-24 · RT · doing · ASan: stack_available() reads a local on ASan's fake stack, so main's linux-extra-asan fails every sandbox test with "0 bytes of stack left" (since ccb6025)
- P1-RT-25 · RT · todo · Measure gem RSS/PSS after a concurrent burst and idle release (does CRuby's malloc_trim reclaim it?); decide on malloc_trim FFI in the engine-binding module; add the idle-release path to a soak; alternate OOM shapes in the 10k soak
- P1-RT-26 · RT · todo · (v0.1) A worker that dies outside guard hangs callers since #54 (one worker): live-worker count via a drop guard, replace a dead worker or fail callers with IslandError::Worker
- P1-RT-21 · RT · todo · Cancel flag inside an island (Sandbox::set_cancel_flag, SandboxError::Cancelled, IslandRunner::render_cancellable), stacked on #47
- P1-RT-22 · RT · doing · Lock-free ForkSafeCell for PRELUDE and the watchdog registry; folded into #47 (v0.1 blocker: 21 of 29,300 forks hung on the watchdog lock)
- P1-RT-23 · RT · todo · Regex deadline gaps: js_regexp_Symbol_replace never polls ('a'.repeat(1<<25).replace(/(?:)/g,'') stops at 1,174 ms vs 250 ms); back-references checked in #47 review and fine
- P1-RT-18 · RT · todo · Follow-ups from the #45 review: the sweep drops a successful run's result (compare it with the clean run's value); Date.parse with an explicit zone at the range end gives an out-of-range value where Chrome gives NaN (fix or COMPAT); memory left by leaked objects at teardown isn't counted (debug counter for the soak); musl fallback only compile-checked
- P1-RT-13 · RT · done (#53) · (#53 for the ASan/flaky tests; interval work folded into P1-RT-14) Core (after #8/#32 merge): lower QuickJS's interrupt-check interval in the vendored engine so expensive built-ins in a loop (e.g. indexOf on a 4 MiB string: TimeLimit after 155 s at a 300 ms limit) end near the limit, with an upstream draft and a hostile case; test robustness: heap_returns_to_baseline_after_reset fails under nightly ASan (TimeLimit; give it a generous limit or relax under sanitizers); time_limit_is_per_call flaky under load; plus #8/#32 review follow-ups
- P1-RT-16 · RT · done (#54) · Memory: measure and cut surimono's PSS overhead over fulgur (pool growth on demand, idle runtime release, shared bytecode, stacks)
- P1-RT-17 · RT · doing · Cold start: built-in bundles shipped as precompiled, version-checked QuickJS bytecode
- P1-RT-11 · RT · done (#37, landed via #39) · Rust harness running fixtures/hostile/manifest.toml against the sandbox and the island runner

### Graphics engineer (crates/gfx)
- P1-GFX-01 · GFX · done · Font registry from caller-supplied font bytes only (TTF, OTF, TTC, WOFF2 if cheap); CSS `font` shorthand parsing; family, weight and style matching; fallback chain for CJK; never system fonts
- P1-GFX-02 · GFX · done · `measureText` from the same shaping the renderer uses (Parley 0.6 as in fulgur): width, actual and font bounding-box ascent and descent; tests against reference widths from fixed fonts
- P1-GFX-03 · GFX · done (#12) · SVG output helpers: deterministic ids, font-family rewritten to registered families, safe inlining into HTML (no script, no external refs)
- P1-GFX-05 · GFX · done (#27) · (#27; merge order #3 → #12 → #27 → #16) Island text to vector outlines in the registered fonts (default; Keep opt-in), because fulgur 0.40 prints inline SVG text with system fonts
- P1-GFX-04 · GFX · done (#16) · Hand-off notes for fulgur: how the same fonts reach fulgur so measured text matches printed text; fixture to prove it

### Conformance researcher (fixtures, wpt, bench, docs/COMPAT.md)
- P1-CF-01 · CF · done · Fixture format (input HTML, data, fonts, meta) and the pipeline: Chrome reference render (pinned Chromium via Playwright), fulgur render, rasterize, perceptual diff, side-by-side PNGs
- P1-CF-02 · CF · done · Test fonts: pinned OFL fonts for Latin and Japanese (e.g. Noto Sans, Noto Sans JP) with licences, used by both Chrome and surimono
- P1-CF-03 · CF · done · The checkpoint-2 fixtures (now 11 fixtures, 12 islands, incl. two charts on one page): ECharts and Vega; simple and dense; Latin and Japanese labels
- P1-CF-04 · CF · done · Hostile-script suite: infinite loop, allocation bomb, deep recursion, microtask flood, huge strings, attempts at fs/network/eval-escape; each with its expected typed error
- P1-CF-05 · CF · done · Real-world suite manifest for phase 1 (ECharts, Vega, Vega-Lite): pinned version, licence, source URL, checksum
- P1-CF-06 · CF · done · Grover benchmark: reference report, time and peak memory for Grover vs surimono+fulgur, table output
- P1-CF-08 · CF · done · Colour, hue and text checks in compare.sh (near-match alone is colour-blind)
- P1-CF-09 · CF · doing · (#35 merged: the run script) the real run itself, once #25/#26 are on main — Checkpoint-2 run once the real pipeline is on main: all 11 fixtures through surimono (outlines) and fulgur, side-by-side PNGs with scores; COMPAT entries for fulgur's system-font SVG text, unsupported outline features (rotate, textLength, textPath, vertical, word-spacing, baseline-shift, decoration, synthetic bold, <style>-only font rules) and Chrome's 1/64 px text positions; wire gfx's svg_inline.mjs and svg_outline.mjs checks into reference.sh
- P1-CF-04b · CF · done · (#36) Hostile follow-ups: real dynamic-import target, narrowed re-entry cases, hostile-chrome.mjs
- P1-CF-10 · CF · done · (#40) Benchmark follow-ups: gem version probe, gem build failure is an error, warm verdict needs full n, pool size recorded
- P1-CF-05b · CF · done · (#41) A missing embedded bundle fails verify.py
- P1-CF-04c · CF · done (#42) · (#42) Hostile follow-ups; blocked: Worker WebSockets escape routeWebSocket
- P1-CF-08b · CF · done (#43) · (#43) Exact font weight and style check
- P1-CF-07 · CF · done (#28) · COMPAT.md generated from test results
- P1-CF-05c · CF · done (#46) · Record #39's Vega bundle hash in the real-world manifest
- P1-CF-12 · CF · review · (#55) Text and font checks read data-surimono-text / data-surimono-face on outline groups (Vega-Lite labels with their own aria-label are false failures otherwise); needed for checkpoint 2
- P1-CF-11 · CF · review · (#50 from main, #51 on #35+#50) Review follow-ups from #28/#35/#42/#43 (unknown fixture names, page-level WebRTC, duplicated labels, positive controls); #35 merges main

### Packaging engineer (crates/surimono, crates/cli, bindings/ruby, .github)
- P1-PKG-01 · PKG · done · CI workflows working end to end (caches, Ruby + libclang for the ext, cargo-deny), pinned action versions
- P1-PKG-02 · PKG · done · Public facade: `Renderer`, `Options` (fonts, limits, renderers), `render_islands(html) -> Result<String>`; typed errors; docs
- P1-PKG-03 · PKG · done · HTML pre-pass: replace `<script type="application/vnd.surimono+json" data-renderer="…">` islands with their SVG; everything else byte-for-byte unchanged
- P1-PKG-04 · PKG · done · Ruby gem: `Surimono.render(html, fonts:, limits:)` mirroring the facade, GVL released while rendering, typed Ruby exceptions, tests on CRuby 3.2 to 4.0
- P1-PKG-05 · PKG · done · Precompiled native gems (x86_64/aarch64 Linux gnu, x86_64/arm64 macOS) via rb-sys cross builds in full.yml
- P1-PKG-06 · PKG · review · CLI: `surimono render in.html -o out.html --font …` (third to cut)
- P1-PKG-01b · PKG · done · (#20) CI scripts: offline fixture checks in the PR gate, nightly fulgur/npm checks, bench cache, hostile results; workflow YAML proposal for Kay (SHA-pinned actions, persist-credentials false)
- P1-PKG-03b · PKG · done · (#22) Pre-pass and facade follow-ups
- P1-PKG-01c · PKG · done (#44) · CI hardening: separate publish job holds the write token, safe status publishing, YAML proposals in docs/ci/ (macos-15)
- P1-PKG-01d · PKG · todo · #44 review follow-ups: take the job from the artifact name and never publish success without a matching needs entry; check the index for link/gitlink entries; reject artifact names starting with a dot; --no-fail-fast in scripts/ci.sh; exit trap that removes the temp worktree
- P1-PKG-10 · PKG · done (#33) · (#33) Gem build carries the vendored QuickJS
- P1-PKG-08 · PKG · review · (#25) Facade on the real IslandRunner and gfx; every island SVG through sanitize_svg (SanitizedSvg newtype); stub removed
- P1-PKG-09 · PKG · review · (#34) THIRD_PARTY_LICENSES.txt for native gems and CLI (cargo-about), incl. bundled JS licences
- P1-PKG-07 · PKG · review · v0.1 release PR with a passing dry-run (cargo publish --dry-run in order, gem build, CHANGELOG), no tag, no publish

### Docs and DX (README.md, docs/guide, examples, CHANGELOG.md)
- P1-DX-01 · DX · doing · README for v0.1: what it is, what it is not, install, a 10-line Rails example, a 10-line Rust example, honest limits
- P1-DX-02 · DX · doing · Guides: Rails with fulgur, Rust; migration notes from Grover for chart-only documents
- P1-DX-03 · DX · doing · Examples that run in CI; wording of error messages reviewed

### DOM engineer (early phase 2 scaffolding; no public API merged before checkpoint 2)
- P1-DOM-01 · DOM · todo · WebIDL fetch and parse from w3c/webref (pinned commit) and a code-generation skeleton producing binding stubs for a handful of interfaces
- P1-DOM-02 · DOM · todo · Spike: id-based handles to blitz-dom 0.2.4 nodes from rquickjs, with a leak test

### Reviewer
- P1-REV-01 · REV · todo · Review every merge; pre-release checklist for v0.1

### Checkpoint
- P1-ORC-01 · ORC · todo · Checkpoint 2 package: 10 side-by-side fixtures, Grover table, hostile-script results, v0.1 release PR with dry-run

## Phase 2: scripts (v0.2, checkpoint 3 on Sun 22 Nov ~11:00), epics

- E2-01 · DOM · todo · Bindings generated from WebIDL for DOM core, events, HTML elements in scope
- E2-02 · DOM · todo · Id-based bindings to blitz-dom: nodes, attributes, tree mutation, serialization
- E2-03 · RT · todo · Event loop on a virtual clock: microtasks, timers fast-forwarded, settle rule (microtasks drained, no timer due in budget, fonts and images resolved)
- E2-04 · DOM · todo · Events, MutationObserver, selectors, classList, dataset, template, URL, TextEncoder, console
- E2-05 · DOM · todo · DOMParser (first to cut)
- E2-06 · PKG · todo · Resource loader with allowlist, network off by default
- E2-07 · DOM · todo · Layout-query policy (zeros or throw)
- E2-08 · PKG · todo · Output: serialized HTML, and the live Blitz document handed to fulgur behind a feature
- E2-09 · CF · todo · WPT slices with expectations: timers and microtasks, dom/nodes, dom/events, domparsing
- E2-10 · CF · todo · Real-world suite: marked, highlight.js, KaTeX auto-render, Alpine.js (second to cut), D3 without measuring
- E2-11 · PKG · todo · musl native gems (third to cut)
- E2-12 · DX · todo · Rails guide for page scripts; checkpoint-3 checklist for Kay's own template

## Phase 3: layout (v0.3, checkpoint 4 on Sun 31 Jan 2027 ~11:00), epics

- E3-01 · DOM · todo · getBoundingClientRect, getClientRects, offset and client sizes from Blitz layout, lazy layout with invalidation on mutation
- E3-02 · DOM · todo · getComputedStyle, matchMedia('print'), document.fonts.ready
- E3-03 · GFX · todo · SVG getBBox and getComputedTextLength
- E3-04 · GFX · todo · Canvas2D recorder to vector paths and text, scoped to Chart.js (anything beyond is second to cut)
- E3-05 · DOM · todo · One-shot ResizeObserver (first to cut)
- E3-06 · CF · todo · WPT slices: svg scripted geometry, css/cssom-view; real-world: Chart.js, D3 axes with measured labels, Mermaid in full DOM mode (third to cut)
- E3-07 · PKG · todo · v0.3 release candidate, pre-release checklist
- E3-08 · DX · todo · Guides and migration docs complete, launch-post drafts
