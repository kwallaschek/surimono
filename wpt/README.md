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
| `results/wpt-results.json` | Generated: what surimono reports, with failure messages ("Results") |
| `runner/` | The runner, a standalone crate ("Runner") |
| `tools/chrome_baseline.mjs` | Writes the baseline in the pinned Chromium |
| `probes/` | Small pages behind `docs/COMPAT.md`'s rows that WPT's slices don't pin, with what Chromium and surimono log for each; `run.mjs` regenerates `results.json` |
| `tools/vendor.py` | Copies the slices from a local WPT checkout at the pinned commit; `--check` verifies them offline |
| `tools/api_needs.py` | Ranks DOM interfaces by how many subtests they unblock, for the DOM engineer |

To vendor again, with a checkout (a sparse, blobless clone of the slices and
`resources/` is enough):

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

`wpt/baseline/chrome.json` is generated, never edited, by
`wpt/tools/chrome_baseline.mjs --from <wpt checkout>`. It runs every URL in the
manifest in the pinned Playwright Chromium's headless shell, on one
`web-platform.test` origin, through the same hook as the runner, with wptrunner's
timeouts (10 s, 60 s for `timeout=long`). wpt.fyi's numbers are not used: they come
from another Chrome build.

Nothing from the checkout runs on the host. A small static server in the script
hands its files to Chromium as data, so it does what WPT's server does statically
(`.any` and `.window` wrappers, `.headers` files, the common `.sub` substitutions),
on plain HTTP with that origin marked secure, and not its Python handlers, apart
from `encoding.py` and `contenttype_setter.py`, redone in the script because 654
subtests hang on them: any other `.py` URL is a 404. A subtest that needs one fails in Chromium and leaves the
denominator. **So the baseline is smaller than a full wptserve run**, and a
rate can only be flattered by what those subtests would have scored. wpt.fyi's
Chrome 156 summary at the same commit is the check on how much that is:
Chrome's passes here against wpt.fyi's are dom/nodes 15267 of 15300, dom/events 748
of 848 (scrolling and input tests that need a real driver), domparsing 909 of 913,
timers and microtasks 34 of 34. Per URL, the baseline's count equals wpt.fyi's or is
lower, never higher. dom/events is the one that matters: it is 11.8% short, so 524
passes would meet 70% of 748 and be 61.8% of 848. Those wpt.fyi counts are in
`slices.toml` (`[wpt_fyi]`), and `compat.py` prints every rate on both denominators;
where the two fall on either side of a target, report both.

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

`wpt/runner/` is a binary on `surimono-dom` and the event loop, reading
`wpt/manifest.toml`. It is a standalone crate like `bench/rust`, with its own
`Cargo.lock`, so a test tool stays out of the library's lockfile and cargo-deny, and
it stays internal: nothing in the facade, CLI or gem.

```
cargo build --manifest-path wpt/runner/Cargo.toml --locked
"$CARGO_TARGET_DIR/debug/surimono-wpt-runner" --root wpt --out wpt/results/wpt-results.json --commit "$(git rev-parse HEAD)"
```

With no `CARGO_TARGET_DIR` the binary is in `wpt/runner/target/debug/`. The fleet's
machine sets one, and uses `scripts/fleet/wcargo.sh` for the build.

For each URL of each `[[test]]`:

1. **Document.** Parse the `.html` file. `.xhtml`, `.xht`, `.xml` and `.svg` tests need an
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
   add_completion_callback((tests, status) => {
     self.__surimonoWptReport = JSON.stringify({
       status: status.status,
       message: status.message,
       tests: tests.map((t) => ({ name: t.name, status: t.status, message: t.message })),
     });
   });
   ```

   The runner reads that global back when the page settles, so the runtime gets
   no test-only native. The hook sends testharness.js's numeric codes; the runner
   maps them (checked against `Test.statuses` and `TestsStatus.statuses` in the
   vendored file):

   | Code | Harness status | Subtest status |
   |---|---|---|
   | 0 | `OK` | `PASS` |
   | 1 | `ERROR` | `FAIL` |
   | 2 | `TIMEOUT` | `TIMEOUT` |
   | 3 | `PRECONDITION_FAILED` | `NOTRUN` |
   | 4 | | `PRECONDITION_FAILED` |

   An unknown code is a value, not a panic: harness `ERROR`, subtest `FAIL`.
   `PRECONDITION_FAILED` isn't a pass, and doesn't count as one.
3. **Scripts and files.** `<script src>` resolves only inside `wpt/tests/`, with
   no network, through a test loader.
4. **Limits and timeouts.** Default sandbox limits per test, except 20 s of time
   instead of 2 s, and a virtual budget of 60 s, wptrunner's long timeout. The
   20 s is real time, and it makes a rate depend less on the build profile, not
   not at all: the six `NodeList-static-length-getter-tampered` tests still hit
   it on the dev profile, and a loaded machine can move a test that is near it. `explicit_timeout`
   turns the harness's own timer off, so the runner decides, wptrunner's model:
   when the page settles with no report, it calls the harness's global `timeout()`,
   and the report arrives as `TIMEOUT` with what finished and the rest `NOTRUN`.
   If no report comes even then, or a sandbox limit is hit, the status is `LIMIT`
   and every subtest of that test counts as failed.
5. **Isolation.** One document and one runtime per test file. A crash can't be
   caught in-process, so the runner shards files into child processes and
   records a dead child as `CRASH` for the test it held. A child still alive
   after 120 s is killed and recorded as `LIMIT`.

`tests/runner.rs` runs these paths on a small hostile manifest: the wrapper,
`timeout()`, the sandbox limits, a page with no harness, and children that abort,
exit, panic, print nonsense or hang (`SURIMONO_WPT_FAULT`, which only that test
sets, because a page can't do those). `--time-limit-ms` and `--wall-limit-ms` keep
it short; the suite itself runs on the defaults above.

### Harness environment

`testharness.js` reads `window.parent`, `window.top`, `window.opener` and `location`
as soon as it loads, and `surimono-dom` has none of them yet. The runner defines
the ones a page lacks, `parent` and `top` as the window, `opener` as null, and a
plain `location` for the test's URL on WPT's default origin. They run at the start
of `testharness.js`'s own script, so the document stays as WPT wrote it, and the
vendored file is unchanged and still checked. A definition goes away by itself when
surimono-dom has the name. `location` here is data with no behaviour, and the gap
is in `COMPAT.md`.

The harness also needs `window` as an event target, with `self` the same object,
`DOMContentLoaded` then `load` after the scripts, `error` and `unhandledrejection` on
`window`, `document.title`, `meta` and `script` lookups, `setTimeout` on the virtual
clock, and `Promise`; surimono-dom has all of those on main.

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
with the target, the commonest causes and every test that doesn't pass in full, and
refuses results from another WPT commit.

`scripts/ci/wpt.sh` runs the suite nightly and publishes `wpt-results.json` to the
`ci-results` branch, with no verdict: the runner exits 0 whatever the tests
report. `wpt/results/wpt-results.json` is the run `docs/COMPAT.md` was last generated
from, committed so that a checkout can regenerate that file without the network and
a change in a rate reads as a diff, test by test. Replace it with a nightly's file,
or a local run of the same commit, when the conformance researcher regenerates
`COMPAT.md`.

## Which tests can't run

A one-document runner has no iframes, new windows, workers or network, and many
tests in `dom/` use iframes. `.sub.html` tests need wptserve's substitution, idlharness
tests fetch `/interfaces/*.idl`, and XML tests need a parser. `api_needs.py` sets
all of these aside so the ranking isn't misled, but they stay in the denominator:
a target is met on all of the slice, or the gap goes in `COMPAT.md` for Kay's OK.
`COMPAT.md`'s cause table shows how much each of these weighs.
