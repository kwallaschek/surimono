# surimono spec (checkpoint 1)

Version 1 · 31 October 2026 · **Approved by Kay at checkpoint 1, 31 Oct 01:19 JST.** Everything here follows the brief; changes from it are listed at the end.

## Name

**surimono** (摺物). Free on crates.io and on RubyGems, checked 31 Oct 2026, as
are `surimono-core`, `-webidl`, `-dom`, `-gfx`, `-islands` and `-cli`. The name
doesn't say what it does, so the one-line description does: "Run a document's
JavaScript without a browser, so browserless HTML-to-PDF engines can print it."
A rename at this checkpoint is one commit (crates, gem, docs).

## Licence

MIT OR Apache-2.0, like Blitz and fulgur. Your LICENSE becomes LICENSE-MIT unchanged, next to LICENSE-APACHE. All dependencies must be permissive,
with one fenced exception: the Servo style crates (Stylo, selectors, servo_arc)
that Blitz needs are MPL-2.0, which is file-level copyleft and fine as an
unmodified dependency. fulgur ships the same.

## Scope

| Phase | Release | Due | In | Cut first if late |
|---|---|---|---|---|
| 1 Islands | v0.1 | Sun 1 Nov | Sandbox; island runner, bytecode cache, runtime pool; text metrics from the renderer's fonts; ECharts and Vega/Vega-Lite to SVG; bring-your-own bundle; HTML pre-pass; Ruby gem and CLI | Mermaid, then Vega, then the CLI |
| 2 Scripts | v0.2 | Sun 22 Nov | Page scripts on blitz-dom; WebIDL-generated bindings; virtual clock and settle rule; network off by default with an allowlisted loader; serialized HTML or the live Blitz document to fulgur | DOMParser, then Alpine.js, then musl gems |
| 3 Layout | v0.3 | Sun 31 Jan | Layout queries from Blitz; getBBox and text measurement; Canvas2D as vectors (Chart.js scope); one-shot ResizeObserver | ResizeObserver, then Canvas2D beyond Chart.js, then Mermaid full DOM |

Never cut: the sandbox limits, determinism, the soak test, box and text measurement,
the Ruby gem. Not in this plan: Python/Node bindings, Windows gems, fetch caching,
WebAssembly, CSS animations, a Rails engine, live websites.

Answers to the brief's open questions (defaults, overturn any):
- The gem bundles ECharts and Vega (about 2 to 3 MB more) so it works out of the box; each is a Cargo feature, so Rust users can leave them out.
- Phases 2 and 3 output serialized HTML first; a fulgur entry point that takes a prepared Blitz document is drafted in `docs/upstream/` for Kay to propose.
- Python bindings come after v0.3.

## Targets

| WPT area | Phase | Pass rate |
|---|---|---|
| html/webappapis timers and microtask queuing | 2 | 90% |
| dom/nodes | 2 | 70% |
| dom/events | 2 | 70% |
| domparsing | 2 | 60% |
| svg scripted geometry (getBBox and related) | 3 | 60% |
| css/cssom-view | 3 | 50% |

- **Memory:** at least 10× lower peak memory than Grover on the reference report; flat memory over 10,000 renders in one process: live heap (Rust and QuickJS) grows less than 64 KiB after warm-up, and resident memory plateaus (reported, with a bound derived from the limits).
- **Speed (new, the brief gave no number):** surimono + fulgur at least 2× faster than Grover per document on the reference report, both cold (new process) and warm (reused process), median of 20 runs. If QuickJS misses this on heavy charts, the number is reported as measured, not hidden.
- **Determinism:** every fixture byte-identical across two renders in one process and one in a fresh process.
- **Safety:** every hostile script (infinite loop, allocation bomb, deep recursion, microtask flood) ends in a typed error within its limit; none crashes or hangs. Default limits: 64 MB heap, 2 s per island, 512 KB JS stack (a whole-document limit may come later).
- **Visual:** the 10 checkpoint-2 fixtures match Chrome's render under a perceptual diff, with text in the same fonts.

## Session cadence

- This weekend (phase 1): the fleet works continuously until checkpoint 2.
- From Mon 2 Nov: a scheduled session resumes the fleet every 4 hours until done.
- A five-line digest every Sunday about 20:00 JST; no reply needed.
- Checkpoints 2 to 4 always wait for you.

## Changes from the brief

1. Blitz is pinned at 0.2.4 (blitz-html 0.2.0), the exact versions in the latest fulgur (0.40.0). Blitz 0.3 is in beta; we move when fulgur does.
2. The MPL-2.0 exception for Stylo above. "Permissive only" can't hold with Blitz in the tree.
3. MSRV is Rust 1.87, the toolchain is pinned to it.
4. A speed target, which the brief left open.
5. scripts/ci.sh runs the same checks as ci.yml locally, so specialists only open a pull request once their branch is green. Chrome references are rendered with a pinned Playwright Chromium, locally and in `reference.yml`.
