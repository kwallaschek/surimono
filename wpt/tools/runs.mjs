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
// file shows what the rule dropped. A result that is the same in every run is
// written as a single run writes it, with no `runs`.

/** One entry per test, from several runs of the same tests (each an array of results). */
export function intersectRuns(runs) {
  if (!Array.isArray(runs) || runs.length === 0) throw new Error('intersectRuns needs at least one run');
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
    const lookup = each.map((r) => new Map((r?.subtests ?? []).map((s) => [s.name, s.status])));
    entry.subtests = names.map((name) => {
      const got = lookup.map((m) => m.get(name) ?? 'NOTRUN');
      if (got.every((s) => s === got[0])) return { name, status: got[0] };
      return { name, status: got.find((s) => s !== 'PASS'), runs: got };
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
