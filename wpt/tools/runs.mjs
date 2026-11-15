// How chrome_baseline.mjs turns several runs of Chromium into one baseline
// (wpt/README.md, "Flaky subtests"). No dependencies, so wpt/tools/test can run it.
//
// A baseline is "what Chromium passes", so a result that varies between runs
// can't count by luck:
// - a test whose harness status isn't the same in every run gets the first
//   status that isn't OK, so compat.py leaves it out whole, as it does a test
//   that times out in a single run;
// - a subtest is PASS only if it is PASS in every run; one that passes in some
//   and not in others gets the first status that isn't PASS ("NOTRUN" where a
//   run didn't report it), so it leaves the denominator.
// Either way the entry keeps what each run said, in run order, as `runs`, so the
// file shows what the rule dropped. A subtest's `message` (a failure's text, where
// the run recorded one) is the one of the run whose status is written. A result that is the same in every run is
// written as a single run writes it, with no `runs`.

// testharness.js's subtest codes (Test.statuses); any other is a FAIL.
const SUBTEST_CODES = { 0: 'PASS', 2: 'TIMEOUT', 3: 'NOTRUN', 4: 'PRECONDITION_FAILED' };

// A failure's text is kept, cut to this many characters, so that a later
// difference can be explained; a pass has none. It counts nothing (compat.py
// reads statuses). The runs the committed baseline is made from predate this, and
// have none.
const MESSAGE_CHARS = 300;

/**
 * A subtest as the baseline keeps it, from testharness.js's report of it: name,
 * status (from the harness's numeric code), and a failure's message.
 */
export function subtestOf(t) {
  const status = SUBTEST_CODES[t.status] ?? 'FAIL';
  const out = { name: t.name, status };
  if (status !== 'PASS' && typeof t.message === 'string' && t.message !== '') out.message = [...t.message].slice(0, MESSAGE_CHARS).join(''); // by code point
  return out;
}

/** One entry per test, from several runs of the same tests (each an array of results). */
export function intersectRuns(runs) {
  if (!Array.isArray(runs) || runs.length === 0) throw new Error('intersectRuns needs at least one run');
  // Results are matched by test id and subtest name, so a name given twice would
  // fold two results into one: refused, in a single run too (none exists today).
  for (const [i, run] of runs.entries()) {
    const ids = new Set();
    for (const r of run) {
      if (ids.has(r.test)) throw new Error(`run ${i + 1} reports ${r.test} twice`);
      ids.add(r.test);
      const names = new Set();
      for (const st of r.subtests ?? []) {
        if (names.has(st.name)) throw new Error(`run ${i + 1}: ${r.test} reports the subtest ${JSON.stringify(st.name)} twice`);
        names.add(st.name);
      }
    }
  }
  if (runs.length === 1) return runs[0];
  const byTest = runs.map((run) => new Map(run.map((r) => [r.test, r])));
  const order = [];
  const seen = new Set();
  for (const run of runs) {
    for (const r of run) {
      if (!seen.has(r.test)) {
        seen.add(r.test);
        order.push(r);
      }
    }
  }
  return order.map((first) => {
    const each = byTest.map((m) => m.get(first.test));
    const statuses = each.map((r) => r?.status ?? 'NOTRUN');
    const { subtests: _subtests, status: _status, message: _message, ...rest } = first;
    const entry = { ...rest };
    if (statuses.every((s) => s === statuses[0])) {
      entry.status = statuses[0];
    } else {
      entry.status = statuses.find((s) => s !== 'OK');
      entry.runs = statuses;
    }
    const names = [];
    const known = new Set();
    for (const r of each) {
      for (const s of r?.subtests ?? []) {
        if (!known.has(s.name)) {
          known.add(s.name);
          names.push(s.name);
        }
      }
    }
    const lookup = each.map((r) => new Map((r?.subtests ?? []).map((s) => [s.name, s])));
    entry.subtests = names.map((name) => {
      const got = lookup.map((m) => m.get(name)?.status ?? 'NOTRUN');
      const same = got.every((s) => s === got[0]);
      // The failure message, if any, of the run whose status is written.
      const from = same ? 0 : got.findIndex((s) => s !== 'PASS');
      const message = lookup[from].get(name)?.message;
      const out = { name, status: same ? got[0] : got[from] };
      if (message !== undefined) out.message = message;
      if (!same) out.runs = got;
      return out;
    });
    const message = each.find((r) => r?.message)?.message;
    if (message && entry.status !== 'OK') entry.message = message;
    return entry;
  });
}

/** The entries `runs` marks: tests and subtests whose result varied, for the log. */
export function varied(merged) {
  const out = [];
  for (const t of merged) {
    if (t.runs) out.push({ test: t.test, subtest: null, runs: t.runs });
    for (const s of t.subtests ?? []) if (s.runs) out.push({ test: t.test, subtest: s.name, runs: s.runs });
  }
  return out;
}
