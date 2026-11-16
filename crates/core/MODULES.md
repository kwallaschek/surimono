# Module scripts and import maps: design (P2-RT-12)

Status: design from the P2-RT-12 spike (branch `p2-rt-12-spike`, which doesn't merge). No public
API is changed by this file or the spike. Measured on 16 Nov 2026.

**The answer first.** Module scripts can run inside the sandbox rules, at about 50 hours of agent
time in 9 steps; the first 5 (34 hours) give a stock Rails 8 importmap page and a Vite-built page. The
prototype already runs both to the end (Turbo, Stimulus and a controller on the Rails page). One
thing must land first: a **use after free in QuickJS-ng** that a page can trigger with a cycle
and one failing import. It crashed the WPT runner with SIGSEGV on
`module/nested-imports.html`. Vendor patch 41 fixes it and goes upstream as a public issue.
Every limit (memory, time, stack, cancel) holds over module parsing, linking and evaluation as a
typed error. The cost is about what a classic bundle costs. Bundling a graph to one classic
script, the cheaper fallback, isn't needed.

## 1. What was measured

Versions: quickjs-ng 0.16.2 (vendored rquickjs-sys 0.14.0, patch level 10), rquickjs 0.14.0, Rust
1.89.0; Rails 8.0.5.2 with importmap-rails 2.2.3, turbo-rails 2.0.23, stimulus-rails 1.3.4 and
propshaft 1.3.2 (installed in a scratch GEM_HOME, 152 MB, deleted afterwards); WPT at the pinned
`8b4b06ff` (sparse checkout in scratch, not added to `wpt/`); quickjs-ng master `947ffe4a` (13
Nov 2026) read for the upstream check. HTML read on 16 Nov 2026: §4.12.1.1 "prepare the script
element", §8.1.4 (script processing model, 8.1.4.8 import map parse results), §8.1.5 (module
specifier resolution, import maps and their processing model), §8.1.6.7 (module host hooks).

The spike tests are `crates/core/src/modules_tests.rs` (`cargo test -p surimono-core --lib p2_rt_12
-- --include-ignored --nocapture`), `crates/surimono/tests/p2_rt_12_modules.rs` and a run of the
existing WPT runner over a scratch manifest. Unless a number says *release*, it comes from a debug
build on the 2-core fleet machine with another agent building alongside it.

### 1.1 The engine

| Case | Result |
|---|---|
| static `import`/`export`, `import * as ns`, cycles (a↔b), one evaluation per URL | correct; `ns[Symbol.toStringTag]` is `Module`; a module imported 3 ways ran once |
| `this` at top level; strict mode | `undefined`; strict (sloppy-only code throws) |
| `import.meta.url` | empty object from the engine; **the host sets `url`** when it compiles the module (the spike does, from the loader) |
| `import()` from a module and from a classic script | works; the referrer is the module's name or the classic script's file name; two imports give the same namespace |
| top-level await | works; a TLA that never settles: microtask-only `run_await` gives `SandboxError::Type` ("still pending"), and on the event loop the page settles `Idle` with the module pending (no hang) |
| missing module | `TypeError: could not load module '<url>'`, thrown when the entry is evaluated, before any module of the graph runs |
| bare specifier not mapped | `TypeError` from the resolver (our message) |
| syntax error in a dependency | `SyntaxError` with the dependency's URL, line and column; nothing in the graph runs |
| link error | `SyntaxError: Could not find export 'x' in module '<url>'` |
| evaluation error | the evaluation promise rejects with the error |
| `with { type: "json" }` | refused by our attribute callback before any fetch. The engine itself has no JSON or CSS modules. A host makes a JSON module with `JS_NewCModule` and one `default` export, so no vendor change is needed. CSS modules need `CSSStyleSheet` construction, which we don't have |

**Hooks.** `JS_SetModuleLoaderFunc2` takes three callbacks: normalize, load and check
attributes. All three are synchronous. Normalize returns a string. Load returns a compiled
`JSModuleDef *` or NULL with an exception, and can't return a pending promise. Static imports
resolve while the importing module compiles. The compile-only `JS_Eval` resolves too, so the
loader runs nested inside the compile, depth first, in source order. Dynamic imports resolve in
the job `import()` queues. A refusal (NULL with an exception) fails a whole static graph before
anything runs, and rejects an `import()`. This suits our loader rule. The loader runs on the
page's worker inside the call that needs it, one fetch at a time, in a deterministic order:
document order of the module scripts, depth first within each graph, then dynamic imports in the
order the page makes them. The worker then asks the caller's thread through the existing
`Message::Need` channel. Module loads therefore happen *while the page runs*, under its time
limit, as frames already do. Classic scripts load before the page runs.

**Limits** (default 64 MiB, 2 s, 512 KiB stack; every row ends in a typed error, and the sandbox
works afterwards):

| Graph | Result |
|---|---|
| chain of 10,000 (and 100,000) | `StackOverflow` after 247 modules, 3.7 ms. The parser's stack check fires in the nested compile, at about 2.1 KiB of stack per level with the default 512 KiB |
| 10,000 imports in one module | runs, 398 ms, 10,000 fetches |
| ring of 10,000 | `StackOverflow` after 247 |
| one 50 MB module | `MemoryLimit` in 1.5 s. With a 1 GiB heap: `TimeLimit` at 2.1 s (compiling 50 MB takes longer than 2 s in debug) |
| endless loop in a dependency; after `await` | `TimeLimit` at 201 ms (200 ms limit) |
| recursion; allocation loop; `import()` ×10⁷ | `StackOverflow`; `MemoryLimit`; `MemoryLimit` in 100 ms |
| a fetch that takes 300 ms (100 ms limit) | `TimeLimit` at 301 ms. The watchdog can't stop a fetch, but the first poll after it can. The facade's 10 s load cap and the loader's own bound still apply |
| host bound of 100 modules | `TypeError: module limit reached` |
| 20,000 modules declared ahead, then evaluated, on an 832 KiB thread (the smallest `check_stack` allows) | **process abort (stack overflow)**: `js_create_module_function` and `js_resolve_module` recurse with no stack check. With patch 42: `StackOverflow` |
| a cycle and a failing import, then another graph through the cycle | **SIGSEGV (use after free)**. With patch 41: a clean `TypeError` both times |

Declaring modules costs more than linear time in the module count: 2,000 take 35 ms, 10,000
take 0.39 s, 20,000 take 2.6 s and 60,000 take 39 s. A module count bound is therefore needed
anyway.

**Memory over renders** (release): 2,000 iterations, each a 50-module graph, a graph that fails,
a dynamic import, and `Sandbox::reset`. RSS stayed at 5 MB from the first iteration to the last,
the heap at 262 KiB after each reset, at 7.9 ms per iteration. Modules live in the context and go
with it. The module host survives rebuilds, and the soak rule holds.

**Bytecode cache and runtime pool.** `crates/islands` runs classic bundles only and never
installs a module host, so nothing interacts. A later per-process cache of module bytecode
(Turbo, Stimulus) would declare modules ahead of time, the shape that needs patch 42.

### 1.2 Cost: 300 modules against one classic bundle of the same code

| | 300 modules (149 KB) | one bundle (110 KB) |
|---|---|---|
| core, release, compile and run, 3 rounds | 11.0–12.7 ms, heap +2,478 KiB | 15.5–16.9 ms, heap +1,632 KiB |
| `render_page`, debug, 20 runs (noisy machine) | min 41–46 ms, median 45–67 ms | min 38–41 ms, median 41–44 ms |
| `render_page` peak RSS (VmHWM, one case per process) | 29.4 MB | 29.2 MB |

The module graph runs as fast as the bundle in the engine and uses about 50 % more heap for
module records, namespaces and bindings. Through `render_page` it costs a few milliseconds for
300 loads, and the same peak memory.

## 2. HTML's model and the page lifecycle

| HTML | Prototype | Design |
|---|---|---|
| `type="module"` (ASCII case-insensitive, trimmed) is a module script; inline and `src` are deferred and run in tree order with `defer` classic scripts, after parsing | yes, in `run_page`'s deferred list | keep |
| `async` module: runs when its graph is ready | keeps its tree position among the non-deferred scripts, the existing `async` departure | keep: deterministic, and the spec allows it since our fetches are instant |
| a classic script after a module runs first | yes | keep |
| `nomodule` classic scripts are skipped; `noModule` reflects | skipped; **reflection still absent** (IDL exclusion of 14 Nov) | reflect it, and add `HTMLScriptElement.supports()` (`classic`, `module`, `importmap` true; `json` false until JSON modules land) |
| `DOMContentLoaded` after the deferred scripts *run*, not after their TLA; `load` after it | yes. A TLA continuation that waits on a timer runs after `load`, because the lifecycle steps are timers queued first (existing behaviour) | keep, and record it in COMPAT |
| `document.currentScript` is null in a module; `this` is undefined; strict | yes | keep |
| `import.meta.url`: an inline module's is the document base URL, an external one's is its URL | yes (see §3 for which URL) | keep; add `import.meta.resolve` (missing: 12 WPT subtests) |
| a fetch failure in the graph: `error` event at the element, nothing runs, no `window.onerror` | **reported as an exception instead** (WPT `fetch-error-*`, `load-error-events*` fail) | the host tells fetch failures from parse and link errors and fires `error` |
| a parse, link or evaluation error: "report an exception" (window `error` and `PageReport.errors`); the page goes on | yes, through the glue's `reportException` | keep |
| `load` event at an external module script | yes | keep |
| a script inserted by script | doesn't run (existing gap, not module-specific: WPT `execorder.html`) | separate task |

## 3. Resolution and loading

**Specifiers.** HTML §8.1.5.1. A URL-like specifier (`/`, `./`, `../`, or an absolute URL)
resolves against the referrer's URL. A bare one goes through the import map, and with no match
it is a `TypeError`. The prototype (`crates/dom/src/modules.rs`) implements "resolve a module
specifier", "resolve an imports match" (exact keys, then `/` prefixes, with the backtracking
check) and scopes, sorted by code units.

**Import maps.** Inline `<script type="importmap">` only. A `src` import map is an error, which
the spec also requires. A key in `integrity` is ignored with a warning, since nothing is fetched
by integrity here. The current text has no "acquiring import maps" flag any more: several import
maps merge (§8.1.4.8 "register an import map", "merge existing and new import maps"). An earlier
map's keys win, and a key for a specifier that something has already resolved is dropped
(`import-maps/multiple-import-maps/`). The prototype merges every map before any module runs.
The design makes this deterministic and close to Chrome: a map registers at its tree position, so
it applies to the module scripts after it, and the resolved-specifier rule is kept.

**The page's URL.** A render's document is `about:blank`, which can't be a base, so the
prototype resolves modules against a synthetic origin, `https://surimono.invalid/`. RFC 6761
reserves `.invalid`, so it can never be a real host. A `<base href>` in the head is respected,
through `Document::base_url`. `import.meta.url` shows the synthetic URL, for example
`https://surimono.invalid/assets/application-bfcdf840.js`.

**What the loader is given.** This is the contract question. Classic scripts keep "the URL is the
attribute as written, with no base applied". A module request needs a resolved URL: `./b.js`
inside `/js/a.js` must become `/js/b.js`, and a bare specifier becomes whatever the map says. The
prototype gives the loader the **path and query** of a URL on the synthetic origin (`/js/b.js`,
`/assets/turbo.min-9fd88cd5.js`), and the full URL of anything else (an absolute map entry, or a
page with a real `<base>`). For Rails and Vite pages this is exactly what the page wrote, so
`Surimono::Rails.assets`' allowlist (`/assets/`) and a hand-made `from_map` both work unchanged.
Classic-script callers see no change. A `src` written relative with no slash
(`<script type=module src="js/app.js">`) reaches the loader as `/js/app.js`. An allowlist entry
`js/` would miss it, which is the one visible break, and it applies only to module scripts, which
never ran before.

**Bounds.** Module loads must share the render's `Fetcher`: 1,000 loads, 10 s of loader time,
and the byte room of `Limits::memory_bytes`. The prototype gives modules a separate counter,
which is a gap. The 1,000-load cap also bounds the module count, which matters for the declare
cost above and for patch 42's depth. No separate depth bound is needed: the stack limit stops a
chain at about 240 levels with the default 512 KiB. The Rails page's graph is 3 levels deep, and Chrome
has no such bound, so this goes in COMPAT. One URL gives one fetch per render, from the cache
keyed by loader URL, and one module per context. A query makes a different module, as in the
spec. No network, ever: the loader is the caller's function.

## 4. Rails and Vite

`javascript_importmap_tags` in a stock `rails new` app (Rails 8.0.5.2 with propshaft, precompiled)
writes three things:

- a `<script type="importmap" data-turbo-track="reload">` with 7 pins, `application`,
  `@hotwired/turbo-rails`, `@hotwired/stimulus`, `@hotwired/stimulus-loading`,
  `controllers/application`, `controllers/hello_controller` and `controllers`, each pointing to a
  digested `/assets/<name>-<digest>.js`;
- 7 `<link rel="modulepreload" href="/assets/…">`;
- `<script type="module">import "application"</script>`.

Every URL is under the asset prefix and is a `.js` file in `public/assets/` or the live Propshaft
pipeline. **`Surimono::Rails.assets` already serves all of them**: it reads `.js` under the asset
paths. No change to the gem is needed except docs. Unverified: the spike drove the page through
`Renderer::render_page` with a Rust loader that maps `/assets/` to `public/`, the same URLs the
gem's loader takes, and not through the gem.

**Result.** The page runs to the end with 0 errors in 54–120 ms (debug). Turbo and Stimulus load,
`window.Turbo.session` exists, Stimulus connects the `hello` controller, and the printed page
reads "Hello World!". The script elements are removed from the output (`keep_scripts` off). The
`modulepreload` links stay in the head, where printers ignore them; the printer-safe output could
drop them.

**What breaks first.** These are globals and APIs a stock app's code or a typical controller
reaches for, listed by name. Each becomes a task. None is specific to modules.

- Missing globals (`typeof window[n] === "undefined"`): `fetch`, `Headers`, `Request`, `Response`,
  `FormData`, `IntersectionObserver`, `ResizeObserver`, `requestIdleCallback`, `matchMedia`,
  `EventSource`, `WebSocket`, `crypto`, `Intl`, `CSS`.
- `HTMLScriptElement.supports` is missing. es-module-shims and others feature-test with it.
- `import.meta.resolve` is missing.
- A `<turbo-frame src="/invoices/1/lines">` stays at its placeholder with no error. Turbo needs
  `fetch` and a server, and the PDF prints "Loading". That goes in COMPAT, and the Rails guide
  says to render frames inline for print.

**Vite-style page (by hand).** `<script type="module" crossorigin src="/assets/index-B1x2.js">`
with relative imports of hashed chunks, a `modulepreload` link and a dynamic `import()` of a chunk
ran to the end with 0 errors. The table rendered, and the chunk drew from
`new URL(import.meta.url).pathname` = `/assets/chart-D5e6.js`.

## 5. Outside truth: WPT

Pinned commit `8b4b06ff`, testharness tests only. The `.worker.js` variants and 1 reftest are left
out. "Needs" means the file needs something we don't have, found by a scan for workers, iframes or
`window.open`, wptserve features (`.py`, `.sub`, `{{…}}`, headers, crossorigin), `fetch`, XHR,
referrer and CSP, JSON or CSS modules, `import.meta.resolve`, or scripts inserted by script. The
rest is "plain". Both columns ran through the existing runner from scratch builds of main and of
the spike, with a manifest of these files.

| Directory | Files (plain / needs) | Subtests on main: pass / reported | Subtests on the spike: pass / reported |
|---|---|---|---|
| `the-script-element/module/` (with `dynamic-import/`, `import-meta/`) | 62 / 60 | 3 / 162 | 112 / 260 (plain: 77 / 116) |
| `import-maps/` | 22 / 27 | 6 / 56 | 12 / 91 (plain: 4 / 44) |
| `the-script-element/microtasks/` | 6 / 9 | 2 / 8 | 6 / 14 (plain: 6 / 11) |
| `the-script-element/json-module/` | 0 / 18 | 0 / 141 | 106 / 141 (see below) |
| `the-script-element/css-module/` | 0 / 18 | 0 / 19 | 0 / 22 |
| **total** | **90 / 132 (222)** | **11 / 386** | **236 / 528** (plain: 87 / 171) |

How to read it:

- `json-module`'s passes are mostly tests that *expect* an import to fail, such as wrong MIME
  types and attribute mismatches. They pass because we refuse every typed import, so they don't
  count toward support.
- Most `import-maps` plain failures (22) are the runner's, not the engine's. Their helper is
  `resources/log.js?pipe=sub&name=X`, and the runner doesn't run wptserve's `sub` pipe on a query,
  so it logs `{{GET[name]}}`. 7 more need `import.meta.resolve`.
- 1 crash, `module/nested-imports.html`, is the use after free (§7).
- Other module failures, by cause:
  - the error event for fetch failures (§2), 4 files;
  - `import defer` (a stage-2 proposal; "meta expected"), 3 tentative files;
  - modules served by `.py` scripts (referrer and credentials tests), about 11;
  - `crypto`;
  - the `import-meta/*.any.js` tests, which the runner wraps in a classic script ("import.meta
    only valid in module code"); CF checks how wptserve wraps them;
  - inserted scripts.

**Proposed conformance target** (for SPEC.md, through the orchestrator). The in-scope set is the
90 plain files plus those that only need `import.meta.resolve` or the `sub` pipe on a query, an estimated 100 files and 190 subtests. CF fixes the list from this scan and gets the Chrome baseline from
wpt.fyi at the pinned commit. The target is **90 % of that set's subtests passing where Chrome
passes**, after steps 1–8. Everything left out goes in COMPAT by directory: workers, iframes,
wptserve features, CSS modules and `import defer`. JSON modules join the set when step 9 lands.

## 6. The `nomodule` reversal

With modules running, `<script nomodule>` must be skipped and `noModule` reflected. That
reverses the lines of 4 Nov ("nomodule is ignored") and 14 Nov ("`noModule` is left out"), and
COMPAT's `nomodule` row goes away.

- `fixtures/real-world/` has no page with a module or a `nomodule` script. The only hit is
  Alpine's list of boolean attributes, so nothing there changes behaviour.
- The `nomodule` probe (`wpt/probes/cases.mjs`) flips to Chrome's result: today we log
  `nomodule ran, classic ran` where Chromium 153 logs `classic ran, module ran`. The prototype's
  lifecycle test gives that order (classic first, then the module, `nomodule` skipped); the
  probe itself wasn't rerun.
- A page that ships both bundles (Vue CLI's modern mode, `@vitejs/plugin-legacy`) now takes the ESM path. In our measurements that path is no heavier: it runs as fast and
  uses about 50 % more heap than the same code as a bundle (§1.2), and legacy bundles usually
  carry polyfills on top. The risk is in what the modern path reaches for: `import.meta.resolve`,
  `fetch` and `HTMLScriptElement.supports`. Hence step 3 adds `supports()` returning true for
  `importmap`, since es-module-shims, for one, decides with it whether to polyfill (through `fetch`).

## 7. Engine changes and unsafe code

**Vendor patches** (in `vendor/rquickjs-sys`, listed in PATCHES.md). Both are bug fixes and go
upstream as one public draft, which the spike branch has as `crates/core/P2-RT-12-UPSTREAM-DRAFT.md`
(quickjs-ng's policy treats AI-found bugs as public; Kay files it). Upstream master `947ffe4a` has
both bugs.

- **41, use after free after a failed graph.** `__JS_EvalInternal` frees only the module whose
  compile failed. A module compiled earlier in the same walk that imports it back (a cycle) keeps
  the pointer. The fix gives each outermost module compile a generation and frees the whole
  generation on failure, which upstream marks with "XXX: should free all the unresolved
  dependencies". About 30 lines. Verified: the minimal repro crashes unpatched and passes patched.
- **42, stack checks** in `js_resolve_module` and `js_create_module_function`, 8 lines. Verified
  as in §1.1.

No vendor feature work is needed. JSON modules can be built host-side with `JS_NewCModule`, and
`import.meta` with `JS_GetImportMeta`.

**Unsafe code.** It stays in one new engine-binding module, `crates/core/src/modules.rs`
(`#[allow(unsafe_code)]`), about 10 blocks: installing the callbacks, the three `extern "C"`
callbacks (each body in `catch_unwind`; a Rust panic can't cross into C), compile with
`import.meta` (`JS_Eval` compile-only, `JS_GetImportMeta`, `JS_DefinePropertyValueStr`), and entry
evaluation (`JS_EvalFunction`). The host is an `Rc<RefCell<…>>` owned by the `Sandbox` and
declared after the context, so it outlives the runtime whose callbacks point at it. It is
re-installed on every rebuild, and a reentrant callback gets a `TypeError` instead of an alias.
rquickjs' own `loader` feature isn't used: its `Loader` can't set `import.meta.url` on a declared
module.

**Errors.** No new `SandboxError` variant is needed. Limits classify as today. Fetch, resolve,
parse and link errors are ordinary exceptions with the module's URL. A rebuild after a limit drops
every module with the context.

**Determinism.** Load order is fixed (§1.1), the fetch is cached per URL, and evaluation is once
per URL per context. There is no clock or randomness in resolution. The synthetic origin is a
constant.

## 8. Public API changes this would need (none made)

1. `ResourceLoader` docs: the module-request rule of §3, a contract change for module scripts only.
2. `ResourceKind::Module`, a new variant, so `PageReport.failed` names module fetch failures. The
   alternative is reusing `Script`.
3. Optional: `Options::page_url(&str)`, with gem `render_page(html, url:)` and CLI `--url`. It sets
   `document.URL` and the base for modules, so `import.meta.url` and `location` can show the app's
   real URL instead of the synthetic origin. Without it the synthetic origin stays an internal
   detail visible only to scripts.
4. Web-facing, not Rust API: `HTMLScriptElement.noModule`, `HTMLScriptElement.supports()`, and
   `import.meta.resolve`.

Core's module host stays `#[doc(hidden)]` scaffolding, as the event loop is. Nothing changes in
`Limits`, `SandboxError`, `PageReport`'s fields, the CLI or the gem beyond item 3.

## 9. Out of scope, and why

- **CSS modules**: they need constructable `CSSStyleSheet` and `adoptedStyleSheets`.
- **Workers and module workers**: no workers.
- **`import defer` and `import source`**: proposals the engine doesn't parse.
- **`modulepreload` semantics**: the graph is fetched on demand at no cost, so preloading has
  nothing to do. The links stay for the printer.
- **Import map `integrity`, CSP, referrer policy and credentials**: no network.
- **Scripts inserted by script** (module or classic): a separate task.
- **A turbo-frame `src`**: needs `fetch` and a server.
- **Bundling module graphs into a classic script**: not needed. It would lose live bindings,
  cycles, `import.meta`, TLA ordering and the error model.

## 10. Step plan

Each step is one branch and one PR, with tests. The first five make a stock importmap Rails page
and a Vite page run. Run two agents at a time, in the pairs shown.

| # | Step | Owner | Hours | Tests | Unblocks |
|---|---|---|---|---|---|
| 1 | Vendor patches 41 and 42, PATCHES.md entries and patch level 11; upstream draft to `docs/upstream/` (orchestrator copies it) | RT | 4 | the two repros (one forks so a regression can't take the suite down); hostile suite unchanged | 2, any module run |
| 2 | Core module host (hidden): callbacks, bounds hooks, `import.meta.url` and `resolve`, entry evaluation, host re-install on rebuild, a TypeError on reentrancy, fetch failures told apart from other errors | RT | 8 | spike cases as tests: graph limits, TLA, errors, attributes; a soak of 10,000 with a graph | 4, 5 |
| 3 | DOM: module and importmap script collection, `nomodule` skipped, `noModule` reflected, `HTMLScriptElement.supports`, deferred ordering, `currentScript` null, the `error` event on fetch failure, `load` | DOM | 8 | `events.rs` (`which_scripts_run` and `the_page_lifecycle…` flip as decided), the probe `nomodule` matches Chrome, WPT `module/` plain | 5 |
| 4 | DOM: import maps (parse, merge, scopes, registration at tree position, resolved-specifier rule), the synthetic origin and `<base>` | DOM | 6 | unit tests from the spec's examples; WPT `import-maps/` plain | 5 |
| 5 | Facade: module fetch through the render's `Fetcher` (shared caps), failures in the report, `ResourceLoader` docs; a Rails fixture (a generated app's page and assets, licence checked) rendered through `Surimono::Rails.assets` in the gem's tests; a Vite fixture | PKG | 8 | page tests, the gem test, page soak with modules | Rails and Vite pages print |
| 6 | WPT slice `modules` (the in-scope list), runner support for modules and the `sub` pipe on queries, Chrome baseline, COMPAT rows | CF | 6 | runner tests; slice in CI | target in SPEC |
| 7 | Rails guide ("External files", "What doesn't work yet": frames, `fetch`, `Intl`), Vite section, CHANGELOG | DX | 3 | examples run in CI | checkpoint |
| 8 | Determinism and soak: each module fixture renders byte-identically twice; `page_soak` with a 300-module graph | RT | 3 | soak, determinism | release |
| 9 | JSON modules (host-built synthetic module, `type: "json"` accepted, `supports("json")` true) | RT | 4 | WPT `json-module/` plain subset | later |

Pairs: (1 ‖ 4), (2 ‖ 6), (3 ‖ 5, which starts once 2 merges), (7 ‖ 8), then 9. Total about 50
hours, 34 of them before Rails and Vite pages print (steps 1–5). The missing globals in §4
(`fetch`, `Intl`, the observers, `matchMedia`) are separate epics that every Chrome-based tool's
users rely on. `Intl` is the largest.

## 11. Decisions for the orchestrator

1. **Loader contract for modules**: path and query on the synthetic origin, else the full URL (§3).
   The alternative is always the absolute `https://surimono.invalid/...`, which breaks every
   existing allowlist.
2. **`nomodule` reversal**: reverses 4 Nov and 14 Nov. Reflect `noModule` and add `supports()`.
3. **Conformance target**: 90 % of about 190 in-scope subtests where Chrome passes (§5).
4. **Async modules** keep their tree position (the existing departure).
5. **Import maps** register at their tree position. Integrity is ignored with a warning, and a
   `src` import map is an error.
6. **Module loads share the render's caps** (1,000 loads, 10 s, the byte room). There is no depth
   bound beyond the stack, and the ~240-level chain limit goes in COMPAT.
7. **`Options::page_url`**: now, or later.
8. **CSS modules are out; JSON modules come later (step 9).**
