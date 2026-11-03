# WPT slices

The phase 2 conformance targets (`docs/SPEC.md`) are measured on five slices of
[web-platform-tests](https://github.com/web-platform-tests/wpt), listed with
their targets in `slices.toml`. Owned by the conformance researcher.

| Path | What |
|---|---|
| `slices.toml` | The pinned WPT commit, and each slice's paths and target |
| `manifest.toml` | Generated: each test, what it needs, sha256 of every vendored file |
| `tests/` | Generated: the vendored files, under WPT's own paths, with WPT's `LICENSE.md` |
| `baseline/chrome.json` | Generated: every subtest Chrome reports at the pinned commit |
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
rate. So `baseline/chrome.json` lists the subtests Chrome reports, and a subtest in
it that surimono never reports counts as failed. The pass rate is passed subtests
over the baseline's subtests that Chrome passes. A subtest Chrome fails is left
out, and listed if the spec says Chrome is wrong (the spec wins for DOM behaviour).

## Runner

The runner sits on `surimono-dom` and the event loop (E2-02, E2-03), as an
integration test in the DOM engineer's crate, reading `wpt/manifest.toml`. It
stays internal: nothing in the facade, CLI or gem.

For each `[[test]]`:

1. **Document.** Parse the `.html` file. For `.any.js` and `.window.js`, build the
   page WPT's server would wrap around it: `testharness.js`, the `// META: script=`
   files, then the test.
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
3. **Scripts and files.** `<script src>` resolves only inside `wpt/tests/`, with
   no network, through a test loader.
4. **Limits.** Default sandbox limits per test. The harness's own timeout runs on
   the virtual clock. A limit hit, or a settled page with no report, is a harness
   error: every baseline subtest of that file counts as failed, never skipped.
5. **Isolation.** One document and one runtime per test file. A crash can't be
   caught in-process, so the runner shards files into child processes and
   records a dead child as `CRASH` for the file it held.

Rust interface:

```rust
/// One vendored test, from wpt/manifest.toml.
pub struct WptTest { pub slice: String, pub file: String, pub needs: Vec<String> }

/// Runs one test file; a failure of any kind is a value, never a panic.
pub fn run_test(test: &WptTest, root: &Path, limits: Limits) -> TestResult;

pub struct TestResult {
    pub status: FileStatus,          // Ok, Error, Timeout, Crash
    pub message: Option<String>,
    pub subtests: Vec<Subtest>,
}
pub struct Subtest { pub name: String, pub status: SubtestStatus, pub message: Option<String> }
// SubtestStatus: Pass, Fail, Timeout, NotRun (testharness.js's four)
```

What it needs from the DOM agent's and runtime agent's work: run a script list in
a document; a settle call that returns when the event loop is idle; a way to install
one native on the global.

## Results

The runner writes `wpt-results.json`, which `compat.py --wpt` reads:

```json
{
  "wpt_commit": "8b4b06ff…",
  "surimono_commit": "…",
  "tests": [
    { "slice": "dom-nodes", "file": "dom/nodes/Node-a.html", "status": "Ok",
      "subtests": [{ "name": "…", "status": "Pass" }] }
  ]
}
```

`compat.py` scores each area against `baseline/chrome.json` and prints a table
with the target, and refuses results from another WPT commit. The results file
comes from CI, like the fixture and hostile results.

## Which tests can't run

A one-document runner has no iframes, new windows, workers or network, and many
tests in `dom/` use iframes. `api_needs.py` sets those aside so the ranking
isn't misled, but they stay in the denominator: a target is met on all of the
slice, or the gap goes in `COMPAT.md` for Kay's OK. How much this weighs is
unknown until the files are vendored.
