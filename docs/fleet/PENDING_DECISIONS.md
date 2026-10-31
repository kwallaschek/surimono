# Pending DECISIONS lines (moved into docs/DECISIONS.md when each PR merges)

## RT batch 1 (#5 #8 #11 #13 #15)
- (superseded by #30: islands uses surimono_gfx::TextMeasurer; no fonts → IslandError::NoFonts; the estimate is test-only)

## PKG batch 1 (#4 #7 #10 #14)

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

## DX batch 1: #18 #19 #23 merged 07:20 Sun; lines moved

## Orchestrator

## CF batch 2 (#2 #9 #17 #24 #28 #31), add on merge
- (compat.py line moved to DECISIONS on #28 merge)

## Open for checkpoint 2 (Kay)
- First real benchmark (CI, before #25 merged, branch release/ci-check-p1-cf-06-pkg08): surimono+fulgur Ruby cold 1,352 ms / warm 686 ms vs Grover 2,153 / 1,581 → 1.6× cold (not met), 2.3× warm (met); PSS 154/179 MB vs 582/704 → 3.8×/3.9× (10× not met; fulgur alone is only 9.4×/8.6×). Grover on chrome-headless-shell is a second baseline (376/496 MB). P1-RT-16 (memory) and P1-RT-17 (precompiled bytecode for cold start) are trying to close the gap.
- 10× memory target: fulgur alone already ~85 MB PSS vs Grover 580 MB (6.8×); on summed RSS 13.7×. Which measure does SPEC mean? Report honestly.
- MSRV: fulgur 0.40 needs rustc 1.89 (krilla 0.7); Rust users pairing surimono with the fulgur crate can't use 1.87 anyway. Plan: raise MSRV to 1.89 at the start of phase 2 (one planned task, full rebuild), not mid-checkpoint.
- Vega speed: scatter 2,000 pts ~1.2 s under load vs 2 s default limit.

## RT sandbox hardening (#32): merged via #39 at 23:30; lines moved; open for Kay: [patch] does not reach crates.io users (publishing surimono-core needs the forked engine under its own crate names).
Known gap: allocation failure during runtime/context creation (real system OOM only) can still crash; not reachable through the memory limit.

## 11:40 · #25 review outcomes
- DECISION 2026-10-31 · A forked child rebuilds its FontSet/TextMeasurer from font bytes along with the pool · A parent thread can hold the measurer's lock at fork time (18 of 200 children hung).
- DECISION 2026-10-31 · Optional max_islands per document (default 1,000) and a cancel path: the Ruby unblock function sets a flag the facade checks between islands · Per-island deadlines don't bound a whole document; Timeout/SIGTERM waited for all islands.
- Open: macos-14 runner image retired 2 Nov; full.yml needs macos-15 (YAML for Kay at checkpoint 2).

## Islands #39 (merged; lines moved)

## RT-15 (#45): merged 00:2x Sun; lines moved.

## PKG batch 3: #25 #26 #34 merged 05:15 Sun; lines moved

## CF batch 4: #50 #51 #55 merged; lines moved

## RT-14 (#47): merged 05:55 Sun; lines moved

## Pending for open PRs (add on merge)

### PKG #38
- 2026-11-01 · The release dry-run links and runs a program against the packaged surimono-core outside the workspace, and requires LICENSE-MIT/LICENSE-APACHE in every released .crate · cargo publish --dry-run only builds an rlib, so it can't see that [patch] doesn't reach crates.io users (#38).
- 2026-11-01 · surimono-webidl and surimono-dom stay publish = false until phase 2 is signed off; the dry-run fails on any publishable crate outside the release list (#38).

## Open for checkpoint 2 (Kay), added 06:00 Sun
- ENGINE DISTRIBUTION (Kay decides): crates.io users of surimono-core get unpatched rquickjs-sys 0.14.0 ([patch] doesn't travel); a linked program fails (undefined surimono_quickjs_patch_level). Options: (1) wait for upstream quickjs-ng/rquickjs releases; (2) publish the fork under our own crate names (3 crates); (3) v0.1 = gem (+ CLI binary) only, Rust crates held. Recommend (3) for v0.1, (2) later.

- Open for checkpoint 2: bench on main 15b530e0 (real pipeline): speed 2.4× cold/warm vs Grover (met); memory 5.8× PSS Ruby / 8.0× Rust (10× not met; fulgur alone 8.7×); vs chrome-headless-shell 1.8–2.3× / 3.8–5.2×.
