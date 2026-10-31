// lib/report.mjs, the verdicts in checkpoint.mjs's index.md:
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { failedChecks, passes, scoresCell, verdict } from "../lib/report.mjs";

// Verdicts report.mjs and compat.py must agree on (test_compat.py reads them
// too, and also compares the two on every combination of their dimensions).
const VERDICTS = JSON.parse(fs.readFileSync(new URL("./verdict-cases.json", import.meta.url), "utf8"));

const ok = (extra = {}) => ({ subject: "x island 0 (printed by chrome)", pass: true, worst_window: 0, worst_colour_window: 0, worst_hue_window: 0, pages: [{ page: 1, pass: true, failed: [] }], ...extra });

test("verdict: a clean score passes; each failed check shows its own number", () => {
  assert.equal(verdict(ok()), "pass");
  assert.equal(verdict(undefined), "not run");
  assert.equal(verdict({ pass: false, error: "surimono exited 3: time limit\nmore", pages: [] }), "**FAIL** (render error: surimono exited 3: time limit)");
  const s = ok({ pass: false, worst_colour_window: 0.4, pages: [{ page: 1, pass: false, failed: ["colour"] }], text: { pass: false, missing: ["1,200"], extra: [] } });
  assert.equal(verdict(s), "**FAIL** (colour 40.0%; text: 1 missing, 0 extra)");
});

test("verdict: the font detail counts mismatches, missing and extra texts", () => {
  // Before P1-CF-11 it showed only mismatches, so an extra-only failure read "font: 0 differ".
  const font = (f) => verdict(ok({ pass: false, font: { pass: false, compared: 2, mismatches: [], missing: [], extra: [], ...f } }));
  assert.equal(font({ extra: [{ key: "East", surimono: "700 normal" }] }), "**FAIL** (font: 1 extra)");
  assert.equal(font({ missing: [{ key: "East", chrome: "400 normal" }] }), "**FAIL** (font: 1 missing)");
  assert.equal(font({ mismatches: [{ key: "Jan", chrome: "400 normal", surimono: "700 normal" }], extra: [{ key: "East", surimono: "700 normal" }] }), "**FAIL** (font: 1 differ, 1 extra)");
});

test("passes is derived like compat.py's, never taken from the top-level pass alone", () => {
  // Review of #35: the headline counted s.pass && !s.error.
  assert.equal(passes(ok()), true);
  const cases = {
    "no pages": ok({ pages: [] }),
    "pages missing": ok({ pages: undefined }),
    "page pass 'false'": ok({ pages: [{ pass: "false", failed: [] }] }),
    "page without pass": ok({ pages: [{ failed: [] }] }),
    "failed text inside a passing score": ok({ text: { pass: false, missing: ["Jan"], extra: [] } }),
    "text pass 'false'": ok({ text: { pass: "false" } }),
    "font that isn't an object": ok({ font: "ok" }),
    "top-level pass 'true'": ok({ pass: "true" }),
  };
  for (const [name, s] of Object.entries(cases)) {
    assert.equal(passes(s), false, name);
    assert.match(verdict(s), /^\*\*FAIL\*\*/, name);
  }
  assert.deepEqual(failedChecks(ok({ pages: [] })), ["no pages"]);
  assert.equal(verdict(ok({ pages: [] })), "**FAIL** (no pages compared)");
  assert.equal(verdict(ok({ pass: false })), "**FAIL** (pass is false)");
  // An absent or null check isn't run, so it can't fail.
  assert.equal(passes(ok({ text: null })), true);
});

test("the shared verdict table", () => {
  for (const c of VERDICTS.cases) {
    assert.deepEqual(failedChecks(c.score), c.failed, c.name);
    assert.equal(passes(c.score), c.pass, c.name);
  }
});

test("scoresCell says ok only when a check's pass is exactly true", () => {
  // Review of #51: "pass": "false" printed "text ok" next to a FAIL verdict.
  assert.equal(scoresCell(ok({ text: { pass: true }, font: { pass: true } })), "0.0% / 0.0% / 0.0% / text ok / font ok");
  assert.equal(scoresCell(ok({ text: { pass: "false" }, font: { pass: 1 } })), "0.0% / 0.0% / 0.0% / text differs / font differs");
  assert.equal(scoresCell(ok({ text: null })), "0.0% / 0.0% / 0.0%");
  assert.equal(scoresCell({ error: "x" }), "—");
  assert.equal(scoresCell(undefined), "—");
});
