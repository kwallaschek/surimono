# Module scripts and import maps: design (P2-RT-12)

Status: design from the P2-RT-12 spike (branch `p2-rt-12-spike`, which doesn't merge), revised on
16 Nov 2026 after the design review on PR #206. No public API is changed by this file or the spike.
Measured on 16 Nov 2026.

**The answer first.** Module scripts can run inside the sandbox rules. The prototype runs a stock
Rails 8 importmap page (Turbo, Stimulus and a controller) and a Vite-style page to the end. The
implementation is 11 steps and about 70 hours of agent time. Modules stay behind a hidden switch,
off by default, until one step turns everything on at once (§10). Main stays green after every
step, and no page renders differently before that step. The page gets a URL of its own from the
start (`Options::page_url`, §3), so resolution, `import.meta.url` and `location` agree when the
caller gives one, and a stand-in that the page can never see is used when it doesn't.

The spike and its review found defects in the engine's module code (a failed graph's cleanup, and
recursion without a stack check on several module paths). Step 1 fixes and audits them in
`vendor/` with tests, before any module loader is installed; module bytecode is not read from a
cache until the bytecode reader's path is covered by that audit. Upstream reports are Kay's to
file. On main today no module loader is installed and nothing compiles source as a module, so
those paths can't be reached; step 1 adds a test that keeps it so while the switch is off.

## 1. What was measured

Versions: quickjs-ng 0.16.2 (vendored rquickjs-sys 0.14.0, patch level 10), rquickjs 0.14.0, Rust
1.89.0; Rails 8.0.5.2 with importmap-rails 2.2.3, turbo-rails 2.0.23 (Turbo 8.0.23), stimulus-rails
1.3.4 (Stimulus 3.2.2) and propshaft 1.3.2 (installed in a scratch GEM_HOME, deleted afterwards);
WPT at the pinned `8b4b06ff` (sparse checkout in scratch, not added to `wpt/`); Chromium
153.0.8010.12 through Playwright 1.63.0, the pinned pair. HTML read on 16 Nov 2026: §4.12.1.1
"prepare the script element", §8.1.4 (script processing model, 8.1.4.8 import map parse results),
§8.1.5 (module specifier resolution, import maps, "merge existing and new import maps"), §8.1.6.7
(module host hooks).

The spike's tests are `crates/core/src/modules_tests.rs`, `crates/surimono/tests/p2_rt_12_modules.rs`
and a run of the existing WPT runner over a scratch manifest. **They print what happens; they don't
assert it.** Every result below is what was seen, and §10 says which step turns it into a test that
asserts. Unless a number says *release*, it comes from a debug build on the 2-core fleet machine
with another agent building alongside it.

### 1.1 The engine

| Case | Result |
|---|---|
| static `import`/`export`, `import * as ns`, cycles (a↔b), one evaluation per URL | correct; `ns[Symbol.toStringTag]` is `Module`; a module imported 3 ways ran once |
| `this` at top level; strict mode | `undefined`; strict (sloppy-only code throws) |
| `import.meta.url` | empty object from the engine; **the host sets `url`** when it compiles the module |
| `import()` from a module and from a classic script | works; two imports give the same namespace |
| top-level await | works; a TLA that never settles: on the event loop the page settles `Idle` with the module pending (no hang) |
| missing module | `TypeError: could not load module '<url>'`, before any module of the graph runs |
| bare specifier not mapped | `TypeError` from the resolver |
| syntax error in a dependency | `SyntaxError` with the dependency's URL, line and column; nothing in the graph runs |
| link error | `SyntaxError: Could not find export 'x' in module '<url>'` |
| evaluation error | the evaluation promise rejects with the error |
| `with { type: "json" }` | refused by the attribute callback before any fetch. A host makes a JSON module with `JS_NewCModule` and one `default` export, so no vendor change is needed. CSS modules need `CSSStyleSheet` construction, which we don't have |

**Hooks.** `JS_SetModuleLoaderFunc2` takes three callbacks: normalize, load and check
attributes. All three are synchronous. Load returns a compiled module or NULL with an exception,
and can't return a pending promise. Static imports resolve while the importing module compiles,
so the loader runs nested inside the compile, depth first, in source order. Dynamic imports
resolve in the job `import()` queues. A refusal fails a whole static graph before anything runs,
and rejects an `import()`. The loader runs on the page's worker inside the call that needs it, one
fetch at a time, in a fixed order: document order of the module scripts, depth first within each
graph, then dynamic imports in the order the page makes them. The worker asks the caller's thread
through the existing `Message::Need` channel, as frames already do. Module loads therefore happen
*while the page runs*, under its time limit. Classic scripts load before the page runs.

**Limits** (default 64 MiB, 2 s, 512 KiB stack; every row ended in a typed error, and the sandbox
worked afterwards):

| Graph | Result |
|---|---|
| static chain of 10,000 (and 100,000), each importing the next | `StackOverflow` after 247 modules, 3.7 ms: the parser's stack check in the nested compile. Not a depth bound (§3) |
| 10,000 imports in one module | runs, 398 ms, 10,000 fetches |
| ring of 10,000 | `StackOverflow` after 247 |
| one 50 MB module | `MemoryLimit` in 1.5 s. With a 1 GiB heap: `TimeLimit` at 2.1 s |
| endless loop in a dependency; after `await` | `TimeLimit` at 201 ms (200 ms limit) |
| recursion; allocation loop; `import()` ×10⁷ | `StackOverflow`; `MemoryLimit`; `MemoryLimit` in 100 ms |
| a fetch that takes 300 ms (100 ms limit) | `TimeLimit` at 301 ms. The watchdog can't stop a fetch, but the first poll after it can |
| host bound of 100 modules | `TypeError: module limit reached` |

The time limit holds during a module's parse and between fetches. Linking doesn't poll it, and
declaring modules costs more than linear time in their number (2,000 in 35 ms, 10,000 in 0.39 s,
20,000 in 2.6 s). So the bound on a graph's size, depth and link time is the render's load cap
(§3), not the stack and not the time limit.

**Memory over renders** (release): 2,000 iterations, each a 50-module graph, a graph that fails,
a dynamic import, and `Sandbox::reset`. RSS stayed at 5 MB from the first iteration to the last,
the heap at 262 KiB after each reset, at 7.9 ms per iteration. Modules live in the context and go
with it.

**Bytecode cache and runtime pool.** `crates/islands` runs classic bundles only and never
installs a module host, so nothing interacts. A per-process cache of module bytecode (Turbo,
Stimulus) waits for step 1's audit of the bytecode reader.

### 1.2 Cost: 300 modules against one classic bundle of the same code

| | 300 modules (149 KB) | one bundle (110 KB) |
|---|---|---|
| core, release, compile and run, 3 rounds | 11.0–12.7 ms, heap +2,478 KiB | 15.5–16.9 ms, heap +1,632 KiB |
| `render_page`, debug, 20 runs (noisy machine) | min 41–46 ms, median 45–67 ms | min 38–41 ms, median 41–44 ms |
| `render_page` peak RSS (VmHWM, one case per process) | 29.4 MB | 29.2 MB |

The module graph runs as fast as the bundle in the engine and uses about 50 % more heap for
module records, namespaces and bindings. Bundling a graph into one classic script isn't needed.

### 1.3 Chromium: where a page without a URL stands

`page.setContent` is how Chrome-based tools usually print an HTML string. Its page is
`about:blank`. Measured in the pinned Chromium (`p2-rt-12b-probe/probe.mjs` in the session
scratchpad; every request answered by a route, no network):

| | `setContent` (`about:blank`) | served at `http://probe.test/reports/123` | `setContent` with `<base href="https://app.example.com/x/">` |
|---|---|---|---|
| `location.href`, `document.URL` | `about:blank` | the page URL | `about:blank` |
| `document.baseURI` | `about:blank` | the page URL | the base |
| inline module's `import.meta.url` | `about:blank` | the page URL | the base |
| `new URL("./logo.png", import.meta.url)` | throws `TypeError` | `http://probe.test/reports/logo.png` | not measured |
| `<script type=module src="chunk.js">` | no request, `error` event | requests `/reports/chunk.js` | requests `https://app.example.com/x/chunk.js` |
| `<script type=module src="/assets/abs.js">` | no request, `error` event | requests `/assets/abs.js` | not measured |
| import map `{"app": "/assets/app-1.js"}`, then `import "app"` | the entry is null ("blocked by a null value"), `TypeError` reported; `import("app")` rejects | loads `/assets/app-1.js` | not measured |
| classic `<script src="classic.js">` | no request, `error` event | requests `/reports/classic.js` | not measured |

So under `setContent` Chrome runs none of a Rails or Vite page's modules, and loads none of its
relative classic scripts either. Served at its real URL it runs all of them. surimono already
departs from `setContent` for classic scripts (the loader gets the attribute as written), and the
design below departs in the same direction for modules, without showing the page anything Chrome
wouldn't. A tool that prints an HTML string at its app URL (Grover's `display_url` option is
described for this; not checked here) matches the served column, which is what `page_url` gives.

## 2. HTML's model and the page lifecycle

| HTML | Prototype | Design |
|---|---|---|
| `type="module"` (ASCII case-insensitive, trimmed) is a module script; inline and `src` are deferred and run in tree order with `defer` classic scripts, after parsing | yes | keep |
| `async` module: runs when its graph is ready | keeps its tree position among the non-deferred scripts, the existing `async` departure | keep: deterministic, and allowed since our fetches are instant. COMPAT row |
| a classic script after a module runs first | yes | keep |
| `nomodule` classic scripts are skipped; `noModule` reflects | skipped; reflection absent | at the flip only (§6) |
| `DOMContentLoaded` after the deferred scripts *run*, not after their TLA; `load` after it | yes. A TLA continuation that waits on a timer runs after `load`, because the lifecycle steps are timers queued first | keep. COMPAT row |
| `document.currentScript` is null in a module; `this` is undefined; strict | yes | keep |
| `import.meta.url`: an inline module's is the document base URL, an external one's is its URL | yes, but showing the stand-in origin | §3: real URLs with `page_url`, never the stand-in without it |
| `import.meta.resolve` | missing (12 WPT subtests) | step 4 |
| a fetch failure in the graph: `error` event at the element, nothing runs, no `window.onerror` | reported as an exception instead | step 3: the host tells fetch failures from parse and link errors and fires `error` |
| a parse, link or evaluation error: "report an exception"; the page goes on | yes | keep |
| `load` event at an external module script | yes | keep |
| a script inserted by script | doesn't run (existing gap, not module-specific) | P2-DOM-19 |

## 3. Resolution and loading

**Specifiers.** HTML §8.1.5.1. A URL-like specifier (`/`, `./`, `../`, or an absolute URL)
resolves against the referrer's URL. A bare one goes through the import map, and with no match
it is a `TypeError`. The prototype (`crates/dom/src/modules.rs`) implements "resolve a module
specifier", "resolve an imports match" (exact keys, then `/` prefixes, with the backtracking
check) and scopes, sorted by code units.

**Import maps.** Inline `<script type="importmap">` only; a `src` import map is an error, as the
spec says. The current HTML text has no "acquiring import maps" flag: several maps merge
("merge existing and new import maps"). A rule with the same scope and specifier as an existing
one is dropped (the earlier map wins), and a rule that would change how an already resolved
specifier resolves is dropped too. The design implements both: a map registers at its tree
position, before the module scripts after it, and the resolved set is tracked per referrer
scope as the spec's examples show. The prototype merged every map before any module ran; step 4
changes that. Departures, each a COMPAT row: a map after an external module script never applies
to that script's graph (in Chrome a slow fetch can let it apply); `integrity` is ignored with a
warning, since nothing is fetched by integrity here; a map a script inserts does nothing
(P2-DOM-19).

**The page's URL** (decided 16 Nov, §11). `Options::page_url(&str)`, gem `render_page(html,
url:)`, CLI `--url`: the document's URL. It is named here and built in step 5.

With `page_url` (for example `https://app.example.com/invoices/1`):

- `document.URL` and `location` are that URL; `document.baseURI` is `<base href>` resolved
  against it, or it. `Document::set_url` already exists and makes `location`, `history`,
  `DOMParser` documents and resolved URL attributes follow.
- Module specifiers resolve against the referrer: an entry `src` or an inline module against
  `baseURI`, a dependency against its importer's URL. `import.meta.url` is the inline module's
  `baseURI` or the module's own URL, as in Chrome's served column (§1.3).
- **Classic scripts too.** `location`, `document.URL` and `baseURI` change for classic scripts in
  the same step that adds the option, because nothing changes for a caller who doesn't pass it.
  The risk is for a page that works today and whose caller then adds `url:`: a script that reads
  `location` (a router, a check on `location.hostname`) takes another branch; `a.href` and
  `img.src` read back as absolute URLs on the app's host, and a script that copies them into the
  page prints absolute URLs; `pushState` to a same-origin path now works where it threw. All of
  these are what Chrome does at that URL. The loader keeps getting a classic `src` as written.

Without `page_url`:

- `document.URL`, `location` and `baseURI` stay as on main: `about:blank`, or an absolute
  `<base href>` for `baseURI`, which is Chrome's `setContent` column.
- For resolution only, the page's origin is the origin of an absolute `<base href>`, or else a
  stand-in, `https://surimono.invalid/` (`.invalid` is reserved by RFC 6761). A relative `src`
  resolves against `/` there.
- **The stand-in is never shown to the page.** The engine is given each module's *loader key*
  (below) as its name, not the stand-in URL, and the resolver maps a key that starts with `/` back
  to the stand-in for resolution. So `import.meta.url`, `import.meta.resolve()`, module names in
  `Error.prototype.stack`, engine error messages, an `error` event's `filename` and
  `PageReport.errors` show `/assets/application-bfcdf840.js`, never a URL on the stand-in. An
  inline module's `import.meta.url` is `about:blank`, as in Chrome. An absolute specifier on the
  stand-in origin is refused with a `TypeError`, so the stand-in is not a usable name either.
- So `import.meta.url` of an external module is a path, not an absolute URL, which HTML never
  gives: a departure with a COMPAT row. `new URL("./logo.png", import.meta.url)`, the pattern
  Vite builds for assets, throws `TypeError` and the error is in the report, as the same line
  throws in Chrome under `setContent` (where the module wouldn't have run at all). That is the
  price of a rule that can't leak: a stand-in URL that a script can read can be written into an
  attribute in a thousand ways, and no rewrite of the output could find them all. The guide says
  to pass `url:` for a page that uses `import.meta.url` or a relative `src`.
- With an absolute `<base href>`, modules on that origin show their real URLs: they are the page's
  own, as in Chrome's base column.

Step 4 has a test that renders the module fixtures and a hostile page without `page_url` (it
writes `import.meta.url`, `import.meta.resolve`, every stack, error message and `filename` it can
reach into the DOM) and asserts that neither the output nor the report contains the stand-in.

**What the loader is given.** Classic scripts keep "the attribute as written". A module request
is a resolved URL, given to the loader as **its path and query when it is same-origin with the
page** (the origin of `page_url`, else of an absolute `<base href>`, else the stand-in), and as the
full URL otherwise. So:

- a stock Rails page gives `/assets/application-bfcdf840.js` with `page_url`, without it, and
  with a `<base>` on the app's host. A caller's `/assets/` allowlist and `from_map` work in all
  three;
- a page with `page_url` on one host and `<base href>` on another (a CDN) gives full URLs for
  modules under the base, which is what Chrome fetches; the caller lists that host, as it already
  does for a classic `src` written as a full URL;
- the one difference between with and without `page_url` is a relative `src` with no leading
  slash: `chunk.js` on a page at `/reports/123` is `/reports/chunk.js` with it and `/chunk.js`
  without it. Rails writes root-relative URLs, and Vite's entry is root-relative with chunks
  relative to the module, which resolve the same either way.

**What `Surimono::Rails.assets` receives.** It strips its configured bases itself (the empty one,
`relative_url_root`, and each form of each asset host) and reads only `.css`, `.js` and `.mjs`
under `public/` and the pipeline's paths (`bindings/ruby/lib/surimono/rails.rb`, `path_of`,
`valid_path?`, `own_entries`). From `javascript_importmap_tags`:

- no asset host: every pin is `/assets/<name>-<digest>.js` (or `<root>/assets/…`), same-origin in
  every case above, so it gets the path, which its allowlist (`<root>/assets/`) lets through and
  `from_public` or `from_propshaft` serves;
- `config.asset_host` set: importmap-rails writes pins as `https://cdn.example.com/assets/…`. That
  is cross-origin with the page, so it gets the full URL, which its host forms in the allowlist
  and `path_of` already cover;
- `modulepreload` links aren't fetched (§9).

So by reading `rails.rb` it serves everything `javascript_importmap_tags` writes, and Q-PKG-09's
"the importmap default … isn't served" should read "isn't run" (modules don't run yet). This is
**unverified through the gem**: the spike drove the Rails page through `Renderer::render_page` with
a Rust loader that maps `/assets/` to `public/`, not through `Surimono::Rails.assets`, and checking
it needs a Rails install this revision didn't make. Step 8 proves it with assertions.

**Bounds: the load cap is the depth bound** (decided 16 Nov). The stack doesn't bound a graph: the
247-level limit above is the parser's check during *nested* compiles, and a chain built leaf first
(successive module scripts, or `await import()`) never nests, so only the module count bounds the
walks over it. The bound is one load cap for the render, shared by classic scripts, stylesheets,
frames and modules, counting every load attempt, failed ones too. The facade's `Fetcher` already
has such a cap for scripts, frames and stylesheets (1,000 loads and 10 s, counted before the loader
is called); module loads join it, and the spike's separate counter, which counted only successes,
goes. Core enforces a cap of its own as well, so its module host is bounded when used without the
facade. The number and its justification are settled in step 1 by measurement on the smallest
stack surimono runs on (a Ruby thread, 1 MiB): every graph shape at the cap (static, leaf-first,
`export *` and re-export chains, rings, wide graphs) must end in a typed error within the time
limit. One URL gives one fetch per render, and one module per context; a query makes a different
module, as in the spec, and counts against the cap. No network, ever: the loader is the caller's
function.

## 4. Rails and Vite

`javascript_importmap_tags` in a stock `rails new` app (Rails 8.0.5.2 with propshaft, precompiled)
writes three things:

- a `<script type="importmap" data-turbo-track="reload">` with 7 pins, `application`,
  `@hotwired/turbo-rails`, `@hotwired/stimulus`, `@hotwired/stimulus-loading`,
  `controllers/application`, `controllers/hello_controller` and `controllers`, each pointing to a
  digested `/assets/<name>-<digest>.js`;
- 7 `<link rel="modulepreload" href="/assets/…">`;
- `<script type="module">import "application"</script>`.

**What was checked.** The page ran through `render_page` with a Rust loader mapping `/assets/` to
`public/`: 0 errors, 54–120 ms (debug). Turbo and Stimulus loaded, `window.Turbo.session` existed,
Stimulus connected the `hello` controller, and the printed page read "Hello World!". These were
printed by the spike, not asserted. Running it through the gem with `Surimono::Rails.assets`, with
assertions on the text, the error count and the loads, belongs to step 8. The `modulepreload` links
stay in the head, where printers ignore them.

**Vite-style page (by hand).** `<script type="module" crossorigin src="/assets/index-B1x2.js">`
with relative imports of hashed chunks, a `modulepreload` link and a dynamic `import()` of a chunk
ran to the end with 0 errors. Its chunk drew from `new URL(import.meta.url).pathname`, which with
§3's rule works with `page_url` and throws without it.

**What a missing global does to a module graph.** A throw during a module's evaluation, such as
`ReferenceError: fetch is not defined` at its top level, rejects that module's evaluation. Every
module that imports it, directly or not, never runs, and a later graph that imports it gets the
same error again. Modules that already ran stay run, and other `<script type=module>` graphs that
don't import it still run. A stock Rails page has one graph (`application` imports Turbo and the
controllers), so one throw at load in Turbo or Stimulus would stop all of the page's JavaScript.
A throw on use (in a listener, in a controller's `connect()`) stops only that call; Stimulus
catches errors from `initialize`, `connect` and actions and logs them (`stimulus.js:1492`, 1505,
392), so one controller fails, not the page.

Where Turbo 8.0.23 (`turbo-rails-2.0.23/app/assets/javascripts/turbo.js`) and Stimulus 3.2.2
(`stimulus-rails-1.3.4/app/assets/javascripts/stimulus.js`, `stimulus-loading.js`) touch the
globals the spike found missing, from an AST walk of the published files plus reading each hit:

| Global | Turbo | Stimulus |
|---|---|---|
| `fetch`, `Headers` | on use: `fetchWithTurboHeaders` (`turbo.js:528`, `new Headers` 529, `window.fetch` 533), for visits, form submits and frames with `src` | none (`fetch` at `stimulus.js:624` is its own map helper) |
| `Request`, `Response` | on use: `url instanceof Response` (4937), in a frame's visit | none |
| `FormData` | on use: `FetchRequest` (610), form submission (1087), `body instanceof FormData` (5520) in a `turbo:before-fetch-request` listener | none |
| `IntersectionObserver` | on use: `new IntersectionObserver` (764) in `AppearanceObserver`, made by `FrameController` (4656) when a `<turbo-frame>` element is constructed. surimono never constructs custom elements, so today it isn't reached; it will be once elements upgrade | none |
| `matchMedia` | on use: `prefersViewTransitions` (2661), during a visit | none |
| `CSS` | on use: `CSS.escape` (4980), on a frame's response | none |
| `EventSource`, `WebSocket` | on use: `turbo-stream-source` connect (5285); at load only `typeof WebSocket` (5529, the bundled Action Cable adapters), which is safe | none |
| `ResizeObserver`, `requestIdleCallback`, `crypto`, `Intl` | none | none |

So nothing in the stock libraries touches a missing global at load, which is why the page ran. App
code can: a controller that formats with `Intl` or reads `matchMedia` fails per controller.

**Proposal: stubs that fail soft, or missing.** Outside truth decides: a global exists only if it
does what Chrome's does in some state Chrome can be in, so a page's own fallback logic sees the
truth. Each line is a task for the orchestrator to number:

- `Headers`, `Request`, `Response`, `FormData` · DOM · real, as pure data (Fetch and XHR specs),
  no network; WPT `fetch/api/headers`, `xhr/formdata` slices.
- `fetch` · DOM, then PKG · present, and every call rejects with `TypeError: Failed to fetch`, as
  Chrome's does offline; through the caller's loader later (a `<turbo-frame src>`), as a separate
  decision. Risk: code that feature-tests `typeof fetch` takes the fetch path and gets the
  rejection, which it must already handle in a browser.
- `CSS.escape`, `CSS.supports` · DOM · real (CSSOM, stylo for `supports`).
- `requestIdleCallback`, `cancelIdleCallback` · DOM · real on the virtual clock, with a fixed
  `timeRemaining()`, after the page's other timers.
- `crypto.getRandomValues`, `crypto.randomUUID` · RT · real from the seeded generator the sandbox
  already has (deterministic); `crypto.subtle` stays missing.
- `matchMedia` · DOM · real, evaluated for print at the page's fixed viewport (phase 3 with layout);
  missing until then.
- `IntersectionObserver` · DOM · missing until phase 3, then real (the initial notification from
  layout); it matters only once custom elements upgrade. `ResizeObserver` is already a phase-3 item.
- `EventSource`, `WebSocket` · none · stay missing: no network, and libraries test for them.
- `Intl` · RT · stays missing; an epic of its own (the engine has none), the largest of these.

## 5. Outside truth: WPT

Pinned commit `8b4b06ff`, testharness tests only. The **in-scope** files are those that need only
what a page without network can have. Out of scope: workers (`.worker.js` and module workers),
iframes and `window.open`, wptserve features (`.py`, `.sub`, `{{…}}`, headers, `crossorigin`),
`fetch`, XHR, referrer and CSP, CSS modules, `import defer` (tentative), scripts inserted by
script (P2-DOM-19), and JSON modules until step 11. The scan found 90 in-scope files (62 in
`module/` with `dynamic-import/` and `import-meta/`, 22 in `import-maps/`, 6 in `microtasks/`) and
132 out of scope.

**Result, in scope only:** 4 of 107 reported subtests pass on main, 87 of 171 in the prototype.
Of the 90 files, main ends 55 OK, 29 timeout, 6 error; the prototype 73 OK, 7 timeout, 9 error and
1 crash (`module/nested-imports.html`, one of the engine defects). "Pass of reported" isn't a rate:
a file that times out reports fewer subtests, so the denominator grows as more runs. Passes that
only expect an import to fail don't count as support: `json-module/`'s are mostly that (wrong MIME
types, attribute mismatches, refused because every typed import is refused), and
`string-compilation-of-promise-result` "failed import" passes on main too. Of 20 in-scope passes
read by the reviewer, 19 need modules to actually run.

What fails in scope, by cause: most `import-maps/` failures are the runner's (its helper
`resources/log.js?pipe=sub&name=X` needs wptserve's `sub` pipe on a query); 7 need
`import.meta.resolve`; 4 need the `error` event for fetch failures; the `import-meta/*.any.js`
tests are wrapped by the runner in a classic script.

**Conformance target** (decided 16 Nov): **90 % of the subtests Chrome passes**, on a list of
in-scope files the conformance researcher freezes before the target goes into SPEC.md, with the
Chrome baseline from the fleet's own five-run process in the pinned Chromium
(`wpt/tools/chrome_baseline.mjs`, `wpt/README.md`), not wpt.fyi. Preconditions: the list is frozen
first, so the denominator can't move with the implementation; the runner supports the `sub` pipe on
a query and wraps `.any.js` module tests as wptserve does (step 6). JSON modules join the list with
step 11.

## 6. The `nomodule` reversal

With modules running, `<script nomodule>` must be skipped and `noModule` reflected. That
reverses the DECISIONS lines of 4 Nov ("nomodule is ignored") and 14 Nov ("`noModule` is left
out"). **It happens in one step, the flip (step 10), together with everything that makes modules
visible** (decided 16 Nov). Until then modules are behind a hidden switch, off by default, and
`nomodule` scripts run as today. The reason: the earlier plan skipped `nomodule` before external
and mapped modules could load, so a page with a module bundle and a `nomodule` fallback, which
works today, would have run nothing in between.

The flip turns on, at once: module and import-map scripts; `nomodule` skipped; `noModule`
reflected; `HTMLScriptElement.supports()` (`classic`, `module`, `importmap` true; `json` false until
step 11); the two DECISIONS reversals; the COMPAT rows of §9; and the guides. Before it, the
real-world suite and the `nomodule` probe (`wpt/probes/cases.mjs`) run both ways, with the switch
off and on, and the PR carries both results. Today the probe logs `nomodule ran, classic ran`
where Chromium 153 logs `classic ran, module ran`.

- `fixtures/real-world/` has no page with a module or a `nomodule` script today (the only hit is
  Alpine's list of boolean attributes). Step 7 adds module entries.
- A page that ships both bundles (Vue CLI's modern mode, `@vitejs/plugin-legacy`) takes the ESM
  path after the flip. That path is no heavier (§1.2). The risk is what it reaches for: a module
  bundle that needs `fetch` at load ran its legacy bundle before and stops after. That is what
  Chrome does, so it is outside truth, and the guide's "What doesn't work yet" says so.

## 7. Engine changes and unsafe code

**The engine.** The spike and its review found defects in the engine's module code (a failed
graph's cleanup, and recursion without a stack check on several module paths). Step 1 fixes and
audits them in `vendor/` with tests, before any module loader is installed; module bytecode is not
read from a cache until the bytecode reader's path is covered by that audit. Upstream reports are
Kay's to file. No vendor feature work is needed: JSON modules can be built host-side with
`JS_NewCModule`, and `import.meta` with `JS_GetImportMeta`.

**Unsafe code.** It stays in one new engine-binding module, `crates/core/src/modules.rs`
(`#[allow(unsafe_code)]`), about 10 blocks: installing the callbacks, the three `extern "C"`
callbacks (each body in `catch_unwind`; a Rust panic can't cross into C), compile with
`import.meta`, and entry evaluation. The host is an `Rc<RefCell<…>>` owned by the `Sandbox` and
declared after the context, so it outlives the runtime whose callbacks point at it. It is
re-installed on every rebuild, and a reentrant callback gets a `TypeError` instead of an alias.
rquickjs' own `loader` feature isn't used: its `Loader` can't set `import.meta.url` on a declared
module. Every entry point checks the switch.

**Errors.** No new `SandboxError` variant is needed. Limits classify as today. Fetch, resolve,
parse and link errors are ordinary exceptions naming the module's loader key. A rebuild after a
limit drops every module with the context.

**Determinism.** Load order is fixed (§1.1), the fetch is cached per URL, and evaluation is once
per URL per context. There is no clock or randomness in resolution; the stand-in is a constant.

## 8. Public API changes

1. `Options::page_url(&str)`, gem `render_page(html, url:)`, CLI `--url` (step 5). Rejected unless
   it parses as an absolute URL that can be a base. A DECISIONS line.
2. `ResourceLoader` docs: the module-request rule of §3, a contract change for module scripts only
   (at the flip).
3. `ResourceKind::Module`, a new variant, so `PageReport.failed` names module fetch failures (at the
   flip; open decision in §11).
4. The hidden switch: a `#[doc(hidden)]` setting on `DomRuntime` and on the facade's `Options`, off
   by default, with no gem or CLI flag, the way the `raikiri` feature and `LayoutPolicy::Real` are
   hidden. The tests and the WPT runner turn it on. It goes at the flip.
5. Web-facing, not Rust API, at the flip: `HTMLScriptElement.noModule`,
   `HTMLScriptElement.supports()`, `import.meta.resolve`.

Core's module host stays `#[doc(hidden)]` scaffolding, as the event loop is. Nothing changes in
`Limits`, `SandboxError` or `PageReport`'s fields.

## 9. Out of scope, and the COMPAT rows

Out of scope, and why:

- **CSS modules**: they need constructable `CSSStyleSheet` and `adoptedStyleSheets`.
- **Workers and module workers**: no workers.
- **`import defer` and `import source`**: proposals the engine doesn't parse.
- **`modulepreload` semantics**: the graph is fetched on demand at no cost, so preloading has
  nothing to do. The links stay for the printer.
- **Import map `integrity`, CSP, referrer policy and credentials**: no network.
- **Scripts inserted by script** (module or classic): P2-DOM-19.
- **A turbo-frame `src`**: needs `fetch` and a server; the guide says to render frames inline.

COMPAT rows, written by CF at the flip, each with the test or probe behind it:

| Departure | Row |
|---|---|
| `async` module keeps its tree position | as the classic `async` row |
| a TLA continuation that waits on a timer runs after `load` | new |
| import maps: a map after an external module script doesn't apply to its graph; `integrity` ignored with a warning; `src` maps are errors (as the spec says, no row); maps inserted by script do nothing | new |
| the load cap bounds a graph's size and depth (Chrome has no such bound); a static chain nested past the stack ends in `StackOverflow` | new |
| `import.meta.resolve` | none if step 4 lands it; a row if the flip ships without it |
| `modulepreload` links aren't fetched | new |
| JSON modules refused until step 11; CSS modules always | new |
| without `page_url`: modules load where Chrome under `setContent` loads none; an external module's `import.meta.url` is a path; a relative `src` resolves against `/` | new |
| `nomodule` is ignored | removed |

## 10. Step plan

Every step is one branch and one PR with tests, merges alone with main green, and changes no page's
output until step 10: everything module-shaped stays behind the switch, and the switch-off tests
(`which_scripts_run`, the lifecycle tests, the `nomodule` probe) keep their current expectations.
Owners: RT, DOM, PKG, CF, DX. Two agents run at a time, never two in the same folders.

| # | Step | Owner | Hours | Needs | Tests (asserting) |
|---|---|---|---|---|---|
| 1 | Engine: fix and audit the module paths in `vendor/` (§7), PATCHES.md, patch level; measure and set the load cap on a 1 MiB thread | RT | 10 | – | the engine cases, asserting the exact error each time, the crash-shaped ones in a child process; every graph shape at the cap ends in a typed error within the time limit; **a test that fails if any module compile path is reachable while the switch is off**: `import()` of a specifier naming an existing script, and of the script's own name, rejects; the engine's module entry points appear only in `crates/core/src/modules.rs`, behind the switch; hostile suite unchanged |
| 2 | Core module host, hidden: callbacks, core's own cap counting attempts, `import.meta` hooks, entry evaluation, re-install on rebuild, `TypeError` on reentrancy, fetch failures told apart | RT | 10 | 1 | §1.1 cases as asserting tests; leaf-first `export *` and re-export chains at the cap with time asserted; a core soak of 10,000 with a graph; switch off: step 1's test |
| 3 | DOM, hidden: module script collection, deferred order, `currentScript` null, the `error` event on fetch failure, `load`; a `DomRuntime` hook that sets the document's URL, for step 5 | DOM | 8 | 2 | with the switch on: ordering and event tests from HTML; WPT `module/` in-scope files through a local runner build; with it off: `which_scripts_run` and the lifecycle tests unchanged |
| 4 | DOM, hidden: import maps (parse, merge with dropped rules, scopes, tree-position registration, resolved set), the page-origin and loader-key rule, the stand-in mapping, `import.meta.resolve` | DOM | 9 | 3 | unit tests from the spec's merge examples; WPT `import-maps/` in scope; **the no-leak test of §3**; with and without a URL and a `<base>` |
| 5 | Facade: `Options::page_url`, gem `url:`, CLI `--url` (live for classic scripts, §3); module loads through the render's `Fetcher` and its shared cap, behind the switch | PKG | 7 | 3 (URL hook; 2 for the module part) | `location`, `document.URL`, `baseURI` with and without `url:`; a page with 1,001 mixed loads (classic, sheet, frame, module) stops at the cap with failures counted; gem and CLI tests |
| 6 | WPT: the frozen in-scope list, the five-run Chrome baseline, runner support for the switch, the `sub` pipe on a query and `.any.js` module wrapping | CF | 6 | list and baseline: –; runner: 3 | runner tests; the slice in CI with the switch on |
| 7 | Fixtures: a Rails page from a generated app (Turbo, Stimulus: version, licence, source URL), a Vite page; Chrome reference renders of both, **served at a URL** (under `setContent` Chrome runs none of them, §1.3); a real-world suite entry with pinned version, licence and URL; THIRD_PARTY rows (orchestrator) | CF | 6 | – | reference.yml renders; manifest checks |
| 8 | Gem and page tests, hidden: the Rails fixture through `Surimono::Rails.assets`, asserting "Hello World!", 0 errors and the load count; the Vite fixture with `url:`; byte-identical twice; a page soak with a 300-module graph | PKG | 5 | 5, 7 | as listed |
| 9 | Guides, prepared on the flip's branch: Rails "External files" (`url:`, module requests), "What doesn't work yet" (frames, `fetch`, `Intl`, the both-bundles case), a Vite section, CHANGELOG | DX | 3 | 8 | examples run in CI |
| 10 | **The flip**, one PR: switch removed; `nomodule` skipped; `noModule`; `supports()`; `ResourceKind::Module`; `ResourceLoader` docs; DECISIONS reversals (orchestrator); COMPAT rows (CF); guides (DX). The real-world suite and the probe run both ways first | DOM, PKG, CF, DX | 6 | 1–9 | the switch-off expectations flip to Chrome's in this PR only; probe `nomodule` matches Chromium; WPT slice at or above the target |
| 11 | JSON modules: host-built module, `type: "json"` accepted, `supports("json")` true | RT | 4 | 10 | WPT `json-module/` in scope |

About 74 hours in all, 70 to the flip. Steps 1–5 can't be shortened much: the engine audit and the
URL rule are the new work the review added.

Pairs (two agents, disjoint folders): (1 RT ‖ 6 CF, list and baseline), (2 RT ‖ 7 CF), (3 DOM ‖
one of §4's RT stub tasks), (4 DOM ‖ 5 PKG), (6 CF, runner ‖ 8 PKG), (9 DX ‖ 11 RT, prepared on a
branch and merged only after the flip), then 10, with DOM, PKG, CF and DX
committing to its one branch in turn and the orchestrator adding the DECISIONS lines. The stub tasks are outside these hours.

## 11. Decisions made

Made by the orchestrator on 16 Nov 2026, from the review's recommendations:

1. **One flip** (b): modules behind a hidden switch through every step; one step turns on modules,
   `nomodule` skipping, `noModule`, `supports()`, the 4 Nov and 14 Nov reversals, COMPAT and the
   guides; the real-world suite and the `nomodule` probe run both ways first.
2. **A page URL from the start** (a): `Options::page_url`, gem `url:`, CLI `--url`; the loader gets
   path and query when same-origin with the page, the full URL otherwise; without it, a stand-in
   for resolution only, never visible to the page (§3). Classic scripts keep the attribute as
   written.
3. **Conformance target** (c): 90 % of the subtests Chrome passes on CF's frozen in-scope list,
   baseline from the fleet's five-run process in the pinned Chromium (§5).
4. **The load cap is the depth bound**: one cap per render shared by classic scripts, stylesheets,
   frames and modules, counting failed attempts, in core and in the facade; the number from step
   1's measurement on a 1 MiB thread.

Still open, each with a recommendation:

1. **`page_url` changes `location` for classic scripts**, in step 5. Recommended: yes, as §3 says;
   only callers who pass it see a change, and it is what Chrome does at that URL.
2. **`import.meta.url` of an external module without `page_url`**: its path (recommended: it can't
   leak and keeps `endsWith` checks working) or `about:blank` for every module (closer to Chrome's
   `setContent`, where nothing runs anyway, but it loses the module's name).
3. **`ResourceKind::Module`** or reusing `Script` for module fetch failures. Recommended: the new
   variant, at the flip, since a caller can then tell which allowlist entry is missing.
4. **The stubs of §4**: recommended as listed, each a task; `fetch` rejecting needs its own
   DECISIONS line because it changes what feature tests see.
5. **Q-PKG-09's wording**: "isn't served" should read "isn't run" (§3), with the gem check in step 8.
6. **Async modules** keep their tree position, and **import maps** register at theirs (§2, §3).
   Recommended: keep, with COMPAT rows.
