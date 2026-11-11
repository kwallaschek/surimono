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
- P1-RT-14 · RT · done (#47) · (#47) Deadline enforcement: watchdog flag checked on every poll; expensive natives poll (indexOf, BigInt cap); flaky core tests; F2/F3 (v0.1 release blockers R4, R5)
- P1-RT-15 · RT · done (#45) · (#45) Allocation-failure crash fixes in the vendored engine; full-coverage OOM sweep; docs stop over-claiming (v0.1 release blockers R6, R7, R8; details private)
- P1-RT-15b · RT · done (#45) · #45 review blockers: sweep covers bind and Promise.any; nits
- P1-RT-20 · RT · done (#90) · quickjs-ng public issue drafts in docs/upstream/quickjs-ng/, one per finding, each with a reproducer on current master and a repro.sh Kay runs to verify (per quickjs-ng's security policy)
- P1-RT-19 · RT · done (#52) · macOS: the determinism test's own TZ probe read 1970, where macOS libc ignores POSIX-rule transitions; probe 2001 and run IANA zones
- P1-RT-24 · RT · done (#57) · (#57) ASan: stack_available() reads a local on ASan's fake stack, so main's linux-extra-asan fails every sandbox test with "0 bytes of stack left" (since ccb6025)
- P1-RT-25 · RT · done · Measure gem RSS/PSS after a concurrent burst and idle release (does CRuby's malloc_trim reclaim it?); decide on malloc_trim FFI in the engine-binding module; add the idle-release path to a soak; alternate OOM shapes in the 10k soak
- P1-RT-26 · RT · done (#71) · (v0.1) A worker that dies outside guard hangs callers since #54 (one worker): live-worker count via a drop guard, replace a dead worker or fail callers with IslandError::Worker
- P1-RT-27 · RT+PKG · done (#73 #77; ASan green on main abd31b70) · Test time-scale factor (env var set by scripts/ci/asan.sh) that multiplies wall-clock limits and bounds in core/islands tests; ASan runs determinism and hostile groups B/C past their bounds; also engine::tests::stack_available_is_plausible fails under ASan (deeper()'s 4 KiB pads live on the fake stack) and hostile alloc-object-graph gets TimeLimit not MemoryLimit (main 8963d8c9)
- P1-PKG-11 · PKG · done (#58) · #25/#26/#34 review follow-ups: drop the stale placeholder_style doc (prepass.rs:411-413); fixtures.rs asserts the island count; CLI docs say an existing read-only output is replaced; about.toml MPL list limited to normal deps (or commented) so real warnings show; root surimono-islands workspace entry default-features=false (ORC) and drop the web_atoms exact pin
- P1-RT-28 · RT · done (#64) · #56 review follow-ups: native-gem jobs and the gem smoke test assert built-ins are precompiled (expose a count); measure the extra source-gem install time; guard test for native `class` in bundles (isESClass under stripping); core build.rs hashes quickjs/ recursively; #57 nit: assert SP vs local agree outside sanitizers
- P1-RT-29 · RT · done · (regexp poll done in #81; left: test-only hook) #47 review follow-ups: hostile.rs times the stop apart from the cleanup (test-only hook); poll per match in js_regexp_Symbol_replace (with P1-RT-23)
- P1-RT-30 · RT · done (#62) · (v0.1) Verify/fix that a forked child never touches the parent's IslandRunner locks (queue Mutex, bundles RwLock, workers Mutex in crates/islands/src/runner.rs) via the facade's pid-claimed slots; fork stress test through the facade; plus #47 review follow-ups: ForkSafeCell checks slot exhaustion before init (no thread leak), doc fix; pid reuse → fork-generation counter (pthread_atfork child handler); expire vs arm/disarm stress test
- P1-RT-31 · RT+GFX · done (#60 #61) · (v0.1 release blocker) LICENSE-MIT/LICENSE-APACHE symlinks and repository/homepage/keywords/categories .workspace in crates/core, crates/islands, crates/gfx
- P1-PKG-12 · PKG · done (#63) · examples/run.sh in CI; TooManyIslands wording; facade slots by fork generation; facade fork stress test; #58 nits
- P1-CF-13 · CF · done (#65) · Re-run checkpoint.mjs and the hostile suite on main after #62/#63; regenerate COMPAT; provenance note on the local-vs-CI Chromium 65 px difference; reference.sh pipe/exit fixes and fail when cargo is missing in CI; determinism.json paths neutral; "59 cases, 60 results" wording
- P1-RT-32 · RT · done (#64) · core forks_never_hang flakes under load (busy thread eval("1") hits its 200 ms limit at load 3 on 2 cores): raise the limit; expire_races_arm_and_disarm asserts its second phase; fork_generation doc mentions _Fork/SYS_fork
- P1-PKG-13 · PKG · done (#67) · Expose precompiled_builtins() in the facade and the gem; native-gem jobs and the gem smoke test assert it equals the built-in count; tests/fork.rs reads the helper's stdout on a thread and kills its process group on timeout; nested-fork case; Ruby gem test: render, Process.fork, render in the child; libc pin → workspace (ORC)
- P1-DX-04 · DX · done (#66) · examples/README: CI now runs run.sh (#63); README Install line wrap; after Kay's engine decision, update install docs and CHANGELOG
- P1-RT-33 · RT · done (#71) · #64 review follow-ups: expire_races_arm_and_disarm hangs instead of failing if an assert fires inside thread::scope (drop guard sets mode=2); its 60 s wait equals the arm budget (arm longer); build.rs symlink cycle guard
- P1-PKG-14 · PKG · done (#69) · #67 review follow-ups: gem.rake aborts on an empty built-in list; fork.rs stale group-id comment or kill before reaping; native-gem.sh wording (only the runner-platform gem is checked); consider builtin_renderers -> &'static [&'static str] before v0.1; switch libc to workspace once ORC adds the pin
- P1-DX-05 · DX · done (#68) · CHANGELOG line for surimono::precompiled_builtins()/builtin_renderers() and Surimono.precompiled_builtins/builtin_renderers (#67); examples/README: macOS/aarch64 jobs run the Rust example tests, not run.sh
- P1-DOM-03 · DOM · done (#74) · #70 follow-ups: test that vendored IDL matches SOURCES.md checksums; parent-cycle check in generate_stubs (typed error); MAX_DEPTH vs small stacks; DESIGN.md says checkpoint 2, not 3
- P1-RT-34 · RT · done (#75) · #71 nits: pool_size() doc (live workers); runner test bound workers_lock().len() <= pool_size + 2; pub const built-in name list in islands for the facade to re-export
- P1-RT-35 · RT · done (#72) · main's full.yml test (macos-14) fails since 4865d3bb: islands echarts a_preloaded_render_matches_a_cold_one sleeps 2 s then asserts preloaded_renders == 1 (got 0 on the slow runner); wait for the preload with a bounded poll instead of a fixed sleep
- P1-RT-21 · RT · done · Cancel flag inside an island (Sandbox::set_cancel_flag, SandboxError::Cancelled, IslandRunner::render_cancellable), stacked on #47
- P1-RT-22 · RT · done (#47) · (#47) Lock-free ForkSafeCell for PRELUDE and the watchdog registry; folded into #47 (v0.1 blocker: 21 of 29,300 forks hung on the watchdog lock)
- P1-RT-23 · RT · done (#81) · Regex deadline gaps: js_regexp_Symbol_replace never polls ('a'.repeat(1<<25).replace(/(?:)/g,'') stops at 1,174 ms vs 250 ms); back-references checked in #47 review and fine
- P1-RT-18 · RT · done · Follow-ups from the #45 review: the sweep drops a successful run's result (compare it with the clean run's value); Date.parse with an explicit zone at the range end gives an out-of-range value where Chrome gives NaN (fix or COMPAT); memory left by leaked objects at teardown isn't counted (debug counter for the soak); musl fallback only compile-checked
- P1-RT-13 · RT · done (#53) · (#53 for the ASan/flaky tests; interval work folded into P1-RT-14) Core (after #8/#32 merge): lower QuickJS's interrupt-check interval in the vendored engine so expensive built-ins in a loop (e.g. indexOf on a 4 MiB string: TimeLimit after 155 s at a 300 ms limit) end near the limit, with an upstream draft and a hostile case; test robustness: heap_returns_to_baseline_after_reset fails under nightly ASan (TimeLimit; give it a generous limit or relax under sanitizers); time_limit_is_per_call flaky under load; plus #8/#32 review follow-ups
- P1-RT-16 · RT · done (#54) · Memory: measure and cut surimono's PSS overhead over fulgur (pool growth on demand, idle runtime release, shared bytecode, stacks)
- P1-RT-17 · RT · done (#56) · Cold start: built-in bundles shipped as precompiled, version-checked QuickJS bytecode
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
- P1-CF-09 · CF · done (#59) · (#35 merged: the run script) the real run itself, once #25/#26 are on main — Checkpoint-2 run once the real pipeline is on main: all 11 fixtures through surimono (outlines) and fulgur, side-by-side PNGs with scores; COMPAT entries for fulgur's system-font SVG text, unsupported outline features (rotate, textLength, textPath, vertical, word-spacing, baseline-shift, decoration, synthetic bold, <style>-only font rules) and Chrome's 1/64 px text positions; wire gfx's svg_inline.mjs and svg_outline.mjs checks into reference.sh
- P1-CF-04b · CF · done · (#36) Hostile follow-ups: real dynamic-import target, narrowed re-entry cases, hostile-chrome.mjs
- P1-CF-10 · CF · done · (#40) Benchmark follow-ups: gem version probe, gem build failure is an error, warm verdict needs full n, pool size recorded
- P1-CF-05b · CF · done · (#41) A missing embedded bundle fails verify.py
- P1-CF-04c · CF · done (#42) · (#42) Hostile follow-ups; blocked: Worker WebSockets escape routeWebSocket
- P1-CF-08b · CF · done (#43) · (#43) Exact font weight and style check
- P1-CF-07 · CF · done (#28) · COMPAT.md generated from test results
- P1-CF-05c · CF · done (#46) · Record #39's Vega bundle hash in the real-world manifest
- P1-CF-12 · CF · done (#55) · Text and font checks read data-surimono-text / data-surimono-face on outline groups (Vega-Lite labels with their own aria-label are false failures otherwise); needed for checkpoint 2
- P1-CF-11 · CF · done (#50, #51) · Review follow-ups from #28/#35/#42/#43 (unknown fixture names, page-level WebRTC, duplicated labels, positive controls); #35 merges main

### Packaging engineer (crates/surimono, crates/cli, bindings/ruby, .github)
- P1-PKG-01 · PKG · done · CI workflows working end to end (caches, Ruby + libclang for the ext, cargo-deny), pinned action versions
- P1-PKG-02 · PKG · done · Public facade: `Renderer`, `Options` (fonts, limits, renderers), `render_islands(html) -> Result<String>`; typed errors; docs
- P1-PKG-03 · PKG · done · HTML pre-pass: replace `<script type="application/vnd.surimono+json" data-renderer="…">` islands with their SVG; everything else byte-for-byte unchanged
- P1-PKG-04 · PKG · done · Ruby gem: `Surimono.render(html, fonts:, limits:)` mirroring the facade, GVL released while rendering, typed Ruby exceptions, tests on CRuby 3.2 to 4.0
- P1-PKG-05 · PKG · done · Precompiled native gems (x86_64/aarch64 Linux gnu, x86_64/arm64 macOS) via rb-sys cross builds in full.yml
- P1-PKG-06 · PKG · done (#26) · CLI: `surimono render in.html -o out.html --font …` (third to cut)
- P1-PKG-01b · PKG · done · (#20) CI scripts: offline fixture checks in the PR gate, nightly fulgur/npm checks, bench cache, hostile results; workflow YAML proposal for Kay (SHA-pinned actions, persist-credentials false)
- P1-PKG-03b · PKG · done · (#22) Pre-pass and facade follow-ups
- P1-PKG-01c · PKG · done (#44) · CI hardening: separate publish job holds the write token, safe status publishing, YAML proposals in docs/ci/ (macos-15)
- P1-PKG-01d · PKG · done (#76) · #44 review follow-ups: take the job from the artifact name and never publish success without a matching needs entry; check the index for link/gitlink entries; reject artifact names starting with a dot; --no-fail-fast in scripts/ci.sh; exit trap that removes the temp worktree
- P1-PKG-15 · PKG · done (#77) · scripts/ci/asan.sh sets SURIMONO_TEST_TIME_SCALE=4 for P1-RT-27
- P1-PKG-16 · PKG · done (#79) · Facade re-exports surimono_islands::BUILTIN_RENDERERS; #76/#77 review nits (publish-results refuses if the index can't be read)
- P1-GFX-06 · GFX · done (#78) · LeakSanitizer in gfx fulgur_layout: stylo 0.8 Box::leaks two thread-local caches per style thread (~13.6 KB/thread); the test runs Blitz on one long-lived thread. Soak/facade must run Blitz on long-lived threads
- P1-PKG-17 · PKG · done (#107) · Symbolize ASan traces in CI with the system LLVM's llvm-symbolizer (asan_env in scripts/ci/lib.sh); follow-ups: overridable /usr/lib prefix with tests, ::warning:: when none found, sudo -n
- P1-PKG-10 · PKG · done (#33) · (#33) Gem build carries the vendored QuickJS
- P1-PKG-08 · PKG · done (#25) · (#25) Facade on the real IslandRunner and gfx; every island SVG through sanitize_svg (SanitizedSvg newtype); stub removed
- P1-PKG-09 · PKG · done (#34) · (#34) THIRD_PARTY_LICENSES.txt for native gems and CLI (cargo-about), incl. bundled JS licences
- P1-PKG-07 · PKG · done (#38) · v0.1 release PR with a passing dry-run (cargo publish --dry-run in order, gem build, CHANGELOG), no tag, no publish

### Docs and DX (README.md, docs/guide, examples, CHANGELOG.md)
- P1-DX-01 · DX · done (#18) · README for v0.1: what it is, what it is not, install, a 10-line Rails example, a 10-line Rust example, honest limits
- P1-DX-02 · DX · done (#19) · Guides: Rails with fulgur, Rust; migration notes from Grover for chart-only documents
- P1-DX-03 · DX · done (#23) · Examples that run in CI; wording of error messages reviewed

### DOM engineer (early phase 2 scaffolding; no public API merged before checkpoint 2)
- P1-DOM-01 · DOM · done (#70) · WebIDL fetch and parse from w3c/webref (pinned commit) and a code-generation skeleton producing binding stubs for a handful of interfaces
- P1-DOM-02 · DOM · done (#80) · Spike: id-based handles to blitz-dom 0.2.4 nodes from rquickjs, with a leak test
- P1-DOM-04 · DOM · done · #80 review nits: serializer keeps xlink:/xml:/xmlns: attribute prefixes; comment data gap (blitz 0.2.4) to COMPAT via CF; noscript raw-text comment; stack-squeeze regression test (JS recursion to the limit then a MAX_DEPTH append); trace the "ERROR: Unexpected token" stderr line in dom tests

### Reviewer
- P1-REV-01 · REV · done (v0.1 pre-release review approved; every merge reviewed) · Review every merge; pre-release checklist for v0.1

### Checkpoint
- P1-ORC-01 · ORC · done (signed off 4 Nov) · Checkpoint 2 package: 10 side-by-side fixtures, Grover table, hostile-script results, v0.1 release PR with dry-run

### Follow-ups found on 4 Nov (main's nightly)
- P1-DOM-05 · DOM · done · `wide_tree_drops_in_linear_time` hits the default 2 s time limit under ASan: scale the limit with SURIMONO_TEST_TIME_SCALE, like the other time-bound tests (asan job red on main since #80)
- P1-CF-30 · CF · done · reference job red on main: gfx Chrome checks in fixtures/tools/reference.sh find no browser (Playwright chromium_headless_shell-1243 missing); COMPAT row for blitz comment data / serializer
- P1-RT-36 · RT · done · Engine patch level 6: segfault on Array subclasses under a memory limit, add_arg check (#95; v0.1 blocker)
- P1-RT-37 · RT · done · Hostile vector B Array.fromAsync accepts either limit (#98)
- P1-RT-38 · RT · done · Trim once per idle period, not once per worker (shared minimum gap); engine.rs doc says the cost grows with the host's heap; gem_memory.rb rescues and counts errors (#89 review)
- P1-PKG-20 · PKG · done (#114) · Ruby 3.2 only: a signal to a main-thread render waits for the whole render when another Ruby thread exists (Ruby 3.2 starts no signal-pipe drainer then; measured 4.04 s vs 0.41 s). Fix A: short-lived pump thread during such renders on 3.2; deterministic test
- P1-DX-06 · DX · done (#110, #113) · (#110) v0.1 CHANGELOG distribution note (gem + CLI from a clone; crates not on crates.io) and README status/install lines for publish day
- P1-PKG-22 · PKG · review (#115 approved) · #114 nits: rescue inside the Ruby 3.2 signal pump; fall back to no pump if Thread.new raises ThreadError; name the pump before it first runs; mention the pump in ffi.rs unblock docs
- P1-DX-08 · DX+CF · done (#118 #119) · README.md:25-27 and COMPAT.md still say the Grover benchmark has no results; cite the committed/CI bench (5.8× Ruby, 8.0× Rust PSS; 2.4× time) once bench/ has the report (with P1-CF-33)
- P1-PKG-23 · PKG · done (#116) · Nightly check of the Rust git-dependency + [patch] recipe; gemspec changelog_uri
- P1-DOM-07 · DOM · todo · Drop the redundant publish lines in crates/dom and crates/webidl after the DOM stack lands
- P1-DX-07 · DX · cut · (no release before v0.3) After Kay tags v0.1.0: pin README, CHANGELOG and the Rust guide to tag = "v0.1.0" / git clone --branch v0.1.0 (main moves to phase 2)
- P1-PKG-21 · PKG · todo · CLI users get no THIRD_PARTY_LICENSES.txt with cargo install: add a `surimono licenses` subcommand (embedded notices) or document where to get them
- P1-PKG-18 · PKG · done (#108) · Wire render_cancellable through the facade: map SandboxError::Cancelled to Error::Cancelled, bypass the island error policy (#85 review)
- P1-PKG-19 · PKG · done (#109) · Run crates/core's main_thread_stack test on x86_64-unknown-linux-musl in CI; wpt/tools checks in scripts/ci/fixtures.sh
- P1-CF-31 · CF · done (#103) · COMPAT entries: Date.parse of non-ISO strings keeps the QuickJS port (V8 gives NaN for CET/CEST), style values not validated, no doctype node, template.content, Attr as a partial Node
- P1-GFX-07 · GFX · todo · crates/gfx/tests/chrome/*.mjs launch with channel 'chromium' so the preload can go
- P1-RT-39 · RT · done (#101) · #85 review nits: doc split, recycle after a queue-cancelled job, doc wording, scaled test time, flag raised and lowered inside one poll
- P1-RT-40 · RT · done · A panic in a native function reaches JS as an exception a script's try can swallow (rquickjs): make any native panic end the render with a typed error, sticky, with a test
- P1-CF-33 · CF · done (#119) · COMPAT.md memory note: target is now ≥5× PSS vs Grover (Kay, checkpoint 2); state the measured 5.8× Ruby / 8.0× Rust and fulgur's own ~8.7× floor
- P1-RT-42 · RT · done (#111) · (#111) Main's linux-extra-asan: LeakSanitizer reports 1,108,992 bytes in 492 indirect leaks at exit of the surimono-core lib test binary (since at least 88cb0ee9); symbolize locally, find and fix
- P1-RT-43 · RT · done (#112) · (#112) Main's test-macos-14 timing flakes: cancel.rs stops_a_microtask_flood (MemoryLimit before Cancelled), hostile.rs group_b_swallowed_interrupts_end; make robust without weakening
- P1-RT-41 · RT · done (#106) · #100 follow-up: first idle worker of a burst must not trim while others hold runtimes (reset last_trim_ms on 0→1 runtimes); #95 follow-ups: add_arg over the parameter limit gives a SyntaxError with a test, PATCHES.md wording, draft 07 reachability; HostPanic message truncated; doc lines from #102's review
- P1-DOM-06 · DOM · done (#93) · Guard every native in crates/dom with Deadline::guard and discards_state (#102 finding 1); required before DOM reaches the facade
- P1-CF-32 · CF · done (#120) · Alpine LICENSE.md in fixtures/real-world/lib once alpinejs/alpine is readable

## Phase 2: scripts (v0.2, checkpoint 3 on Sun 22 Nov ~11:00), epics

> Checkpoint 2 is signed off; phase-2 public API may land once its task is reviewed.
- P2-ORC-01 · ORC · done (#117) · MSRV 1.89: toolchain, rust-version, docs, clippy
- P2-RT-02 · RT · done (#121) · a_burst_after_a_long_gap_trims_once flaky on GitHub (main red): deterministic test; two workers idling together could both trim
- P2-RT-01 · RT · done (#122) · #99 review follow-ups: wait tokens unique across reset() (generation in the token, test), wording at event_loop.rs:63 and :49; PATCHES.md item 25 says the JS_MAX_LOCAL_VARS path wasn't run (now it was: SIGSEGV from JS on unpatched master); generator `{} = {}` variant of too_many_destructured_parameters_is_a_syntax_error; sandbox.rs:462 rewrap
- P2-CF-01 · CF · done (#119) · COMPAT lines from #99: await sleep(0) vs Chrome 153, NUL and lone surrogate in string handlers, the "promise never settled" quote at COMPAT.md:126
- P2-DX-01 · DX · done (#118) · Docs after #108: a cancel stops the running island (README:258, rust.md:195, islands.md:164, rails.md:331, surimono.rb:130 first-load caveat); CHANGELOG Unreleased: MSRV 1.89, cancel, Ruby 3.2 pump
- E2-01 · DOM · done (#93) · Bindings generated from WebIDL for DOM core, events, HTML elements in scope
- E2-02 · DOM · done (#93 #94 #96 #97) · Id-based bindings to blitz-dom: nodes, attributes, tree mutation, serialization
- E2-03 · RT · done (#91 #99) · (a merged #91; b: rAF frames grouped, external waits vs timers, classic-script string handlers) Event loop on a virtual clock: microtasks, timers fast-forwarded, settle rule (microtasks drained, no timer due in budget, fonts and images resolved)
- E2-04 · DOM · done (#104 #105; URL, TextEncoder and console in E2-13 if missing) · Events, MutationObserver, selectors, classList, dataset, template, URL, TextEncoder, console
- E2-05 · DOM · done (#139) · DOMParser (first to cut)
- E2-13 · DOM · done (#131 #132 #134 #135 #136 #140 #142) · WPT wins after the stack: new Document(), document.implementation, iframes for selector/characterSet tests (decide scope), XML parser (decide), window.frames, MouseEvent, named access on Window, on* handlers, document.currentScript, document.body on frameset pages, style on SVG elements, error events for timer and microtask exceptions
- P2-PKG-01 · PKG · todo · CI job building wpt/runner and publishing wpt-results.json; renderer.rs:328 doc says up to twice the time limit per bundle from source and forked first renders; surimono.rb first-load caveat
- P2-RT-04 · RT · done (#126) · Proxy ownKeys duplicate check quadratic and unpolled: sorted copy, engine patch level 8
- P2-RT-05 · RT · done (#127) · Unhandled rejections reported; a limit error inside a promise ends the call typed (needed by #104)
- P2-DOM-02 · DOM · done (#128) · Deterministic normalize test, selector weight, frameset document.body
- P2-DX-02 · DX · done (#129) · README, guides and CHANGELOG on main say nothing is released yet (no gem on RubyGems, Kay 5 Nov); install from a clone until v0.3
- P2-RT-03 · RT · done (#125) · The QuickJS parser never polls the time limit: a 60 MiB source with a 50 ms limit took 8.8 s before TimeLimit (#124 probe). Test eval(hugeString) and page scripts; add parser polls in the vendored engine or a source-size cap in the sandbox
- P2-CF-02 · CF · done (#123 #133 #146) · COMPAT requests from the DOM stack (reports for #94 #96 #97 #104 #105): cloneNode(document), surrogate offsets, selector gaps and caps, style validity, nomodule, window.event, isTrusted, listeners' microtask checkpoint, on* gaps, unhandledrejection, ErrorEvent fields; spec-vs-Chrome row on important-declaration order; dom/events denominator 748 vs 848; vendor.py .xml gap; wpt/README notes (#123 review)
- P2-DOM-04 · DOM · done (#144) · Main red at 89343c8f: selector caps overflowed the stack on ASan and aarch64 debug; byte-scan depth count bypassed by comments and strings; d3 libm ulp on macOS
- P2-DOM-05 · DOM · done (#148) · crates/dom needs from #143: DomRuntime::cancel_flag (cancel stops a running script), Document::to_html with a byte cap and stop check, html5ever quadratic attributes (60,000 attributes took 13.8 s under 500 ms); check("noscript-message") in real_world.rs; parse_xml content types
- P2-RT-06 · RT · done (#149) · unhandledrejection with HTML's "notify about rejected promises" timing (hook in notify_rejections); a microtask checkpoint callable from a native; export stack_available; fixed libm (22 wrappers plus pow and hypot; cost about half a day; `libm` crate or msun; never V8's ieee754.cc)
- P2-PKG-02 · PKG · done (#147) · wpt/runner fmt, clippy and tests in CI
- E2-14 · DOM · todo · Remaining WPT gaps by subtests: domparsing needs 60% (39.8%): append/prepend/before/after/replaceWith HTML and trustedTypes are new unstable API (Kay's call at checkpoint 3), history.pushState 90, Range 39, createContextualFragment 35; dom/events: createProcessingInstruction 6, document.cloneNode/createHTMLDocument 11; selectors :lang :enabled :disabled :checked :target about 100
- P2-RT-08 · RT · done (#150) · Rare hang in a forked child: std's thread_info lock and ASan's quarantine locks; plain pthread_create for host threads
- P2-DOM-06 · DOM · todo · Follow-ups: layout policy for unbound members (elementsFromPoint, scroll methods, getBBox); noscript text in DOMParser documents written raw; one long token isn't interrupted by the stop (80 MB of &amp; took 625 ms against 50 ms); Object.create(window).name setter; SVG handler parameter `evt`; `<svg onload>`; AbortSignal.timeout listeners checkpoint; XML and frames in tests/soak.rs
- E2-06 · PKG · done (#143; images fire no load events) · Resource loader with allowlist, network off by default. Caps its waits: at most 1,000 waits started and 10 s real time per render, then external_abandon_all, fail the rest without opening a wait, let the page settle; delivers completions in a fixed order (#99 review)
- E2-07 · DOM+PKG · done (#132, #143) · Layout-query policy (zeros or throw)
- E2-08 · PKG · done for serialized HTML (#143); live hand-off waits on a fulgur entry point · Output: serialized HTML, and the live Blitz document handed to fulgur behind a feature. Flush style attributes per element before the live hand-off with a test (nothing else does until phase 3; pick the no-namespace attribute); bound the printer's whole stylo parse (many capped attributes add up, <style> text unscreened, var() carries grid strings); screen script-set style text; text moved out of <style> stays empty in blitz; the hand-off must drop <noscript> because fulgur parses with scripting off (#93 #94 #104 reviews)
- E2-09 · CF · done (#123 #145 #151 #156 #165) · WPT slices with expectations: timers and microtasks, dom/nodes, dom/events, domparsing (first run #123: 73.5%, 18.8%, 10.8%, 13.5%; PKG job to publish wpt-results.json still needed)
- E2-10 · CF · doing · Real-world suite: marked, highlight.js, KaTeX auto-render, Alpine.js (second to cut), D3 without measuring
- P2-ORC-02 · ORC · review (#175) · Root pins for html5ever, html-escape, stats_alloc, cssparser, url
- E2-15 · DX · done (#157) · README, rust.md, rails.md, CHANGELOG for the page path; checkpoint-3 checklist
- E2-10b · CF · done (#158: 2 of 6 pass; the rest equal their fulgur floor) · Print comparison for the real-world pages through render_page and fulgur against Chrome
- E2-16 · DOM · done (#160-#164) · URL family, encoding, base64, structuredClone, navigator, HTML collections, tables, forms, TreeWalker, styleSheets
- P2-PKG-03 · PKG · done (#159) · MPL crates in about.toml; fork guard; page-check in CI
- P2-DOM-07 · DOM · done (#166) · A throwing string timer in WebIDL order; restore noscript escaping in template contents (#159 is on main); DESIGN.md stale lines at ~456 and ~565; #160 and #161 review leftovers (resizable buffers, define checks, a&a&a quadratic, number edge cases, focus with no browsing context)
- P2-CF-05 · CF · done (#165) · Extra WPT areas, COMPAT regenerated, pages in COMPAT
- P2-CF-06 · CF · done (#167) · Rerun traversal and css/cssom after #162 #163; README WPT numbers to Docs
- E2-11 · PKG · done (#170) · musl native gems (third to cut)
- E2-12 · DX · done (#157) · Rails guide for page scripts; checkpoint-3 checklist for Kay's own template
- P2-GFX-01 · GFX · done (#168) · reference.yml `chrome` red since 3 Nov: svg_outline.mjs draws Chrome's text unhinted and in grayscale
- P2-PKG-04 · PKG · done (#169) · Nightly ASan red since 7 Nov: the printer parses on one long-lived thread; the page soak measures a quiet renderer
- P2-RT-07 · RT · done (#171) · Correctly rounded Math from CORE-MATH, the same bits on every platform (engine patch level 10)
- P2-RT-09 · RT · done (#172) · ASan test that forks with the parent idle and renders in the child
- P2-DX-03 · DX · done (#173) · README WPT numbers at 22153052; all four phase-2 targets met
- P2-DX-04 · DX · review (#174) · "What doesn't work yet" lists after #160 to #166; CHANGELOG new globals
- P2-RT-10 · RT · doing · Red main (full.yml at 3b4303ae): arm64 `forks_never_hang` lost 1 child of 3000; ASan `limits_are_typed_and_the_renderer_carries_on` ran out of time. Cause, fix, and a stress run that says why a child failed
- P2-DX-05 · DX · todo · #174 review follow-ups: CHANGELOG puts the 2 MiB cap on `URL` alone (a standalone `URLSearchParams` has none); rails.md:481 and rust.md:427 name both errors (missing globals give `ReferenceError`, missing members `TypeError: not a function`)

## Quality track (from Kay's directive of 11 Nov: it works, prints on par with Chrome-based tools, faster, less memory)

> Ranked above everything except a red main. Quality means the printed page against Chrome's.
- Q-CF-01 · CF · doing · Printer matrix, first part running: fulgur 0.40.0 against the newest fulgur. Second part waits for Kay's OK to install and run the other printers on the fleet machine (ironpress, rustyprint, forme): the session's safety check refused it. Print every checkpoint-2 and checkpoint-3 page, all fed surimono's output; score against the committed Chrome references, with time and peak PSS per printer; report in fixtures/printers/
- Q-CF-02 · CF · todo · Page-path benchmark: the six real-world pages through render_page + printer against Grover and Ferrum (time cold and warm, summed PSS), in bench/ beside the islands report
- Q-GFX-01 · GFX · review (#176) · Root-cause the print gaps of fixtures/checkpoint-3/NOTES.md in fulgur 0.40 and blitz 0.2.4 sources: `<svg display:block>` ignoring width/height, single-side border colour, list markers, synthesized italic, inline `code` background, `<pre>` height, KaTeX vertical-align and inline-block struts, `@font-face` URLs. Per gap: where, the smallest patch, a test, whether the newest fulgur/Blitz has it; drafts in docs/upstream/ (by the orchestrator's assignment)
- Q-PKG-01 · PKG · todo · One-call print path behind the facade's `fulgur` feature (Rust `print_pdf`, gem `to_pdf`, CLI `surimono pdf`) with the same fonts and page size for measuring and printing; design first, build after Q-CF-01 and Q-GFX-01 say which printer and which patches
- Q-PKG-02 · PKG · doing · Printer-safe output: rewrites surimono applies to its own output so stock fulgur 0.40/0.41 prints it as Chrome prints the original, each one neutral in Chrome under the cascade and proven on a fixture: block `<svg>` sizes from attributes (B1), a single painted border side's colour (F1), inline boxes of a multi-line paragraph under `<body>` (F5), the exact content width (F6), a trailing newline in `<pre>` and a trailing `<br>` (B2). An option turns them off. Root causes in docs/upstream (branch q-gfx-01, #176)
- Q-ORC-02 · ORC · done · fulgur's direction: Raikiri (github.com/fulgur-rs/raikiri, MIT OR Apache-2.0) is being built "to replace fulgur's layout engine"; in 0.41 it is opt-in (`--engine raikiri`), Blitz is the default, no timeline. It keeps Parley and Taffy. E3-00 puts layout queries behind an internal seam; see DECISIONS 2026-11-11
- Q-GFX-02 · GFX · todo · After Q-ORC-01: the printer fixes that have no workaround, as patches with tests against the pinned versions: list markers drawn as shapes with Blink's gap (F2), synthesized italic (F3), inline backgrounds (F4), ligature text ranges (F7), parley's content width at a forced break (P1)
- P2-CF-07 · CF · todo · From Q-GFX-01: COMPAT rows (the flex-item row is parley's P1; "fractional line heights" didn't reproduce; add early line wrap F6 and ligature text F7); checkpoint-3/NOTES.md: the lost `E = mc²` is fulgur's inline-box gap, not KaTeX's
- Q-ORC-01 · ORC · todo · Decide from Q-CF-01 and Q-GFX-01: patched print path shipped by us, or upstream only; one DECISIONS line with the numbers

## Phase 3: layout (v0.3, checkpoint 4 on Sun 31 Jan 2027 ~11:00), epics

> Started 11 Nov: all four phase-2 targets are met. Public API still waits for checkpoint 3.
- E3-00 · DOM · doing · Design spike in crates/dom/DESIGN.md: lazy layout on blitz-dom 0.2.4 (resolve, invalidation on mutation, page width and fonts from options), which queries Blitz answers, cost per query, what a style-attribute flush needs; a prototype getBoundingClientRect behind a doc(hidden) switch with tests against Chrome probes

- E3-01 · DOM · todo · getBoundingClientRect, getClientRects, offset and client sizes from Blitz layout, lazy layout with invalidation on mutation
- E3-02 · DOM · todo · getComputedStyle, matchMedia('print'), document.fonts.ready
- E3-03 · GFX · todo · SVG getBBox and getComputedTextLength
- E3-04 · GFX · todo · Canvas2D recorder to vector paths and text, scoped to Chart.js (anything beyond is second to cut)
- E3-05 · DOM · todo · One-shot ResizeObserver (first to cut)
- E3-06 · CF · todo · WPT slices: svg scripted geometry, css/cssom-view; real-world: Chart.js, D3 axes with measured labels, Mermaid in full DOM mode (third to cut)
- E3-07 · PKG · todo · v0.3 release candidate, pre-release checklist
- E3-08 · DX · todo · Guides and migration docs complete, launch-post drafts
