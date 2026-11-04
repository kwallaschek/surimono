# WPT slices

The phase 2 conformance targets (`docs/SPEC.md`) are measured on five slices of
[web-platform-tests](https://github.com/web-platform-tests/wpt), listed with
their targets in `slices.toml`. Owned by the conformance researcher.

| Path | What |
|---|---|
| `slices.toml` | The pinned WPT commit, and each slice's paths and target |
| `manifest.toml` | Generated: each test, what it needs, sha256 of every vendored file |
| `tests/` | Generated: the vendored files, under WPT's own paths, with WPT's `LICENSE.md` |
| `baseline/chrome.json` | Generated: every test and subtest Chrome reports at the pinned commit, with statuses ("Baseline") |
| `tools/vendor.py` | Copies the slices from a local WPT checkout at the pinned commit; `--check` verifies them offline |
| `tools/api_needs.py` | Ranks DOM interfaces by how many subtests they unblock, for the DOM engineer |

**State:** `manifest.toml`, `tests/` and `baseline/` don't exist yet. This session
has no read access to web-platform-tests/wpt, so nothing is vendored; the commit in
`slices.toml` came from wpt.fyi's run metadata. With a checkout:

```
git -C <wpt> checkout 8b4b06ff53a47f1041e0e59f894d9e929963904c
python3 -I wpt/tools/vendor.py --from <wpt>
python3 -I wpt/tools/vendor.py --check
python3 -I wpt/tools/api_needs.py
```

## What counts as an expectation

A WPT test holds its own expectations, so nothing here restates them. The
manifest records which files run and their hashes; every subtest is expected to
pass. A subtest we fail stays failing, and shows up in `docs/COMPAT.md`. There are no
per-subtest "expected fail" lists to edit, and `vendor.py --check` fails on any
changed test file.

**The denominator comes from Chrome.** Subtest names exist only once a test runs,
and a test that crashes early would otherwise shrink the total and raise our
rate. The pass rate is passed subtests over the subtests Chrome passes, so a
subtest in the baseline that surimono never reports counts as failed. A subtest
Chrome fails is left out, and listed in `COMPAT.md` if the spec says Chrome is wrong
(the spec wins for DOM behaviour).

Scoring one test, in `compat.py`:

| Chrome's status | surimono's status | Counts |
|---|---|---|
| anything but `OK` | any | nothing: left out |
| `OK` | `OK` | each subtest Chrome passes, passed if ours is `PASS` |
| `OK` | `TIMEOUT` | the same, since the harness reports what finished; but if every one passed, the test failed, because something didn't finish |
| `OK` | `ERROR`, `CRASH`, `LIMIT`, or not reported | every subtest Chrome passes, all failed |

A test with no subtests counts once: passed when Chrome's status and ours are `OK`.

## Baseline

`wpt/baseline/chrome.json` is generated, never edited. The conformance researcher
writes `wpt/tools/chrome_baseline.mjs` once the tests are vendored, because it
needs the real `testharness.js`. It runs every test in the pinned Playwright
Chromium, offline, through the same `testharnessreport.js` hook as the runner.
wpt.fyi's numbers are not used: they come from another Chrome build.

```json
{
  "schema": 1,
  "wpt_commit": "8b4b06ff…",
  "chromium": "153.0.8010.12",
  "tests": [
    { "slice": "dom-nodes", "test": "dom/nodes/Node-a.any.html?x", "file": "dom/nodes/Node-a.any.js",
      "status": "OK", "subtests": [{ "name": "…", "status": "PASS" }] },
    { "slice": "dom-nodes", "test": "dom/nodes/no-subtests.html", "file": "dom/nodes/no-subtests.html",
      "status": "OK", "subtests": [] }
  ]
}
```

`compat.py --wpt` refuses a baseline with no entry for a slice, or without a URL
`manifest.toml` lists: a partial baseline is how a rate gets inflated.

**Test ids.** A test is keyed by the path WPT serves it at, without the leading
slash and with its variant, which is wpt.fyi's id minus the slash. `.any.js` runs
as `.any.html` and `.window.js` as `.window.html`. `// META: variant=` and
`<meta name="variant">` make one file several tests, each with its own results.
`// META: global=` with no window scope means no test here, and `vendor.py` leaves
the file out. `manifest.toml` records each file's `urls`; `file` is what the
loader reads.

## Runner

The runner sits on `surimono-dom` and the event loop (E2-02, E2-03), as an
integration test in the DOM engineer's crate, reading `wpt/manifest.toml`. It
stays internal: nothing in the facade, CLI or gem.

For each URL of each `[[test]]`:

1. **Document.** Parse the `.html` file. `.xhtml`, `.xht` and `.svg` tests need an
   XML parser, which surimono-dom doesn't have: they report `ERROR` until it does.
   For `.any.js` and `.window.js`, build the page WPT's server would wrap around it:
   `self.GLOBAL` (`isWindow()` true, `isWorker()` and `isShadowRealm()` false) set
   first, then `testharness.js`, `testharnessreport.js`, `<div id=log>`, the
   `// META: script=` files, then the test. Serve each `urls` entry with its
   query string, at the path WPT uses, because some subtest names embed the URL.
2. **Harness.** Run the real, unmodified `resources/testharness.js`. Never a
   rewrite: a shim could pass what the real one fails. The runner supplies
   `/resources/testharnessreport.js`, the embedder hook WPT reserves for this:

   ```js
   setup({ output: false, explicit_timeout: true });
   add_completion_callback((tests, status) =>
     __surimonoWptReport(JSON.stringify({
       status: status.status,
       message: status.message,
       tests: tests.map((t) => ({ name: t.name, status: t.status, message: t.message })),
     })));
   ```

   `__surimonoWptReport` is one native installed by the runner, for tests only.
   The hook sends testharness.js's numeric codes; the runner maps them:

   | Code | Harness status | Subtest status |
   |---|---|---|
   | 0 | `OK` | `PASS` |
   | 1 | `ERROR` | `FAIL` |
   | 2 | `TIMEOUT` | `TIMEOUT` |
   | 3 | `PRECONDITION_FAILED` | `NOTRUN` |
   | 4 | | `PRECONDITION_FAILED` |

   An unknown code is a value, not a panic: harness `ERROR`, subtest `FAIL`.
   These codes are from memory of testharness.js and must be checked against
   the vendored file. `PRECONDITION_FAILED` isn't a pass, and doesn't count as one.
3. **Scripts and files.** `<script src>` resolves only inside `wpt/tests/`, with
   no network, through a test loader.
4. **Limits and timeouts.** Default sandbox limits per test. `explicit_timeout`
   turns the harness's own timer off, so the runner decides, wptrunner's model:
   when the page settles with no report, it calls the harness's global `timeout()`,
   and the report arrives as `TIMEOUT` with what finished and the rest `NOTRUN`.
   If no report comes even then, or a sandbox limit is hit, the status is `LIMIT`
   and every subtest of that test counts as failed.
5. **Isolation.** One document and one runtime per test file. A crash can't be
   caught in-process, so the runner shards files into child processes and
   records a dead child as `CRASH` for the test it held.

Rust interface:

```rust
/// One test to run: a URL of a `[[test]]` in wpt/manifest.toml.
pub struct WptTest { pub slice: String, pub url: String, pub file: String, pub needs: Vec<String> }

/// Runs one test; a failure of any kind is a value, never a panic.
pub fn run_test(test: &WptTest, root: &Path, limits: Limits) -> TestResult;

pub struct TestResult {
    pub status: TestStatus,          // Ok, Error, Timeout, PreconditionFailed, Crash, Limit
    pub message: Option<String>,
    pub subtests: Vec<Subtest>,
}
pub struct Subtest { pub name: String, pub status: SubtestStatus, pub message: Option<String> }
// SubtestStatus: Pass, Fail, Timeout, NotRun, PreconditionFailed
```

What the harness needs from E2-02, E2-03 and E2-04, beyond running a script list,
a settle call that returns when the event loop is idle, and installing one native:

- `window` as an event target, with `self` the same object;
- `DOMContentLoaded`, then `load`, after the scripts: tests can't complete without it;
- `error` and `unhandledrejection` on `window`, or uncaught failures vanish;
- `location`, `document.title`, and `meta` and `script` element lookups;
- `setTimeout` on the virtual clock, and `Promise`.

This list is from memory of testharness.js too; check it when the file is vendored.

## Results

The runner writes `wpt-results.json`, which `compat.py --wpt` reads:

```json
{
  "wpt_commit": "8b4b06ff…",
  "surimono_commit": "…",
  "tests": [
    { "slice": "dom-nodes", "test": "dom/nodes/Node-a.html", "status": "OK",
      "subtests": [{ "name": "…", "status": "PASS" }] }
  ]
}
```

`compat.py` scores each area against `baseline/chrome.json` and prints a table
with the target, and refuses results from another WPT commit. The results file
comes from CI, like the fixture and hostile results.

## Which tests can't run

A one-document runner has no iframes, new windows, workers or network, and many
tests in `dom/` use iframes. `.sub.html` tests need wptserve's substitution, idlharness
tests fetch `/interfaces/*.idl`, and XML tests need a parser. `api_needs.py` sets
all of these aside so the ranking isn't misled, but they stay in the denominator:
a target is met on all of the slice, or the gap goes in `COMPAT.md` for Kay's OK.
How much this weighs is unknown until the files are vendored; one large
idlharness file could dominate `domparsing`.
