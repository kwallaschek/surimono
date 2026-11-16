// chrome-defaults.mjs's verdict, on answers made up here: no Chromium.
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import test from "node:test";
import { EXPECTED, judge } from "../chrome-defaults.mjs";

const all = (v) => ({ "text-autospace": { root: v, initial: v, text: v } });

test("Chromium 153's answer passes", () => {
  assert.deepEqual(EXPECTED.map((e) => [e.property, e.expected]), [["text-autospace", "no-autospace"]]);
  assert.deepEqual(judge(all("no-autospace"), "Chromium 153.0.8010.12"), []);
});

test("a Chromium that computes the spec's `normal` fails, naming the value, the rule and what to do", () => {
  const failures = judge(all("normal"), "Chromium 999.0.0.0");
  assert.equal(failures.length, 3);
  assert.match(failures[0], /^Chromium 999\.0\.0\.0 computes text-autospace as "normal" on root, not "no-autospace"\./);
  assert.match(failures[0], /`:root\{text-autospace:no-autospace\}`/);
  assert.match(failures[0], /crates\/surimono\/src\/print\/safe\//);
  // Where it moved is named: only the element set to `initial` here.
  const one = judge({ "text-autospace": { root: "no-autospace", initial: "normal", text: "no-autospace" } });
  assert.equal(one.length, 1);
  assert.match(one[0], /on initial,/);
});

test("a Chromium that no longer knows the property fails too", () => {
  const failures = judge({ "text-autospace": {} });
  assert.equal(failures.length, 1);
  assert.match(failures[0], /gave no value for text-autospace/);
  assert.match(judge({})[0], /gave no value/);
});
