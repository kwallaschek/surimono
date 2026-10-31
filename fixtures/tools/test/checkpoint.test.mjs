// checkpoint.mjs's failure paths, with the stand-in CLI (test/fake-surimono.mjs)
// in modes that never reach Chrome or fulgur:  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const tools = path.resolve(here, "..");
const fake = path.join(here, "fake-surimono.mjs");
const FIXTURE = "echarts-bar-simple-latin";

function run(env, out, extra = []) {
  return spawnSync(process.execPath, [path.join(tools, "checkpoint.mjs"), "--out", out, "--surimono", fake, ...extra, FIXTURE], {
    encoding: "utf8",
    env: { ...process.env, FAKE_SURIMONO_FROM: "", FAKE_SURIMONO_FAIL: "", FAKE_SURIMONO_LEAK: "", ...env },
  });
}
const made = [];
const tmp = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "checkpoint-test-"));
  made.push(d);
  return d;
};
test.after(() => {
  for (const d of made) fs.rmSync(d, { recursive: true, force: true });
});
const readJson = (f) => JSON.parse(fs.readFileSync(f, "utf8"));

test("--out inside fixtures/ is refused", () => {
  const r = run({}, path.join(tools, "..", "report", "x"));
  assert.equal(r.status, 2, r.stderr);
  assert.match(r.stderr, /inside fixtures/);
  assert.equal(fs.existsSync(path.join(tools, "..", "report", "x")), false);
});

test("a failed render writes a failing, stamped score for every comparison", () => {
  const out = tmp();
  const r = run({ FAKE_SURIMONO_FAIL: FIXTURE }, out);
  assert.equal(r.status, 0, r.stderr);
  for (const d of ["island-0-chrome", "island-0-fulgur", "page"]) {
    const s = readJson(path.join(out, FIXTURE, d, "score.json"));
    assert.equal(s.pass, false);
    assert.match(s.error, /time limit/);
    assert.match(s.producer.version, /fake-surimono/, "every score says which CLI made it");
    assert.equal(s.producer.cli, fake);
  }
  assert.match(fs.readFileSync(path.join(out, "index.md"), "utf8"), /\*\*FAIL\*\* \(render error: surimono exited 3/);
  // compat.py refuses a stand-in's results instead of reporting them.
  const c = spawnSync("python3", ["-I", path.join(tools, "compat.py"), "--fixtures", out, "--out", path.join(out, "COMPAT.md")], { encoding: "utf8" });
  assert.equal(c.status, 1, c.stdout + c.stderr);
  assert.match(c.stderr, /stand-in/);
});

test("a render that depends on the time zone is caught and fails the run", () => {
  const out = tmp();
  const r = run({ FAKE_SURIMONO_LEAK: "TZ" }, out);
  assert.equal(r.status, 1, "a determinism failure exits non-zero");
  assert.match(r.stderr, new RegExp(`not byte-identical.*${FIXTURE}`));
  const d = readJson(path.join(out, FIXTURE, "determinism.json"));
  assert.equal(d.deterministic, false);
  assert.deepEqual(d.runs.map((x) => x.TZ), ["UTC", "Asia/Tokyo"]);
  // The page has no islands, so every comparison fails without Chrome.
  assert.match(readJson(path.join(out, FIXTURE, "island-0-chrome", "score.json")).error, /0 islands, the fixture has 1/);
});

test("a render that depends on the working directory or locale is caught too", () => {
  for (const leak of ["cwd", "LANG"]) {
    const out = tmp();
    assert.equal(run({ FAKE_SURIMONO_LEAK: leak }, out).status, 1, leak);
    assert.equal(readJson(path.join(out, FIXTURE, "determinism.json")).deterministic, false, leak);
  }
});

test("byte-identical renders pass the determinism check", () => {
  const out = tmp();
  const r = run({}, out);
  assert.equal(r.status, 0, r.stderr);
  assert.equal(readJson(path.join(out, FIXTURE, "determinism.json")).deterministic, true);
});

test("floors come from a baseline run, never from constants", () => {
  const out = tmp();
  const base = tmp();
  fs.writeFileSync(path.join(base, "baseline.json"), JSON.stringify({ fulgur: "0.40.0", rows: [{ fixture: FIXTURE, page: { worst_window: 0.012 }, islands: [{ worst_window: 0.034 }] }] }));
  run({ FAKE_SURIMONO_FAIL: FIXTURE }, out, ["--baseline", base]);
  const md = fs.readFileSync(path.join(out, "index.md"), "utf8");
  assert.match(md, /at most 1\.2% on a page and 3\.4% on an island/);
  run({ FAKE_SURIMONO_FAIL: FIXTURE }, out);
  assert.match(fs.readFileSync(path.join(out, "index.md"), "utf8"), /wasn't measured in this run/);
  // A baseline with no numeric scores says so instead of "-Infinity%".
  fs.writeFileSync(path.join(base, "baseline.json"), JSON.stringify({ fulgur: "0.40.0", rows: [{ fixture: FIXTURE, page: { error: "x" }, islands: [{ error: "x" }] }] }));
  run({ FAKE_SURIMONO_FAIL: FIXTURE }, out, ["--baseline", base]);
  const md2 = fs.readFileSync(path.join(out, "index.md"), "utf8");
  assert.doesNotMatch(md2, /Infinity/);
  assert.match(md2, /at most not measured \(no scores in the baseline\) on a page/);
});

test("the second render starts in a later second of the wall clock", () => {
  const out = tmp();
  // fake-surimono with FAKE_SURIMONO_LEAK=SECOND appends the current Unix second.
  const r = run({ FAKE_SURIMONO_LEAK: "SECOND" }, out);
  assert.equal(r.status, 1, "a render that prints the second differs");
  assert.equal(readJson(path.join(out, FIXTURE, "determinism.json")).deterministic, false);
});
