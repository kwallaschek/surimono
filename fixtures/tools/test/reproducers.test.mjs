// reproducers.mjs: what it refuses, and one page through Chrome and a stand-in printer
// (the pinned Chromium and poppler are needed for that one; without them it is skipped).
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const tools = path.resolve(here, "..");
const repo = path.resolve(tools, "..", "..");
const fakeFulgur = path.join(here, "fake-fulgur.mjs");
const REPRODUCER = "F1-border-side-colour";

const made = [];
const tmp = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "reproducers-test-"));
  made.push(d);
  return d;
};
test.after(() => {
  for (const d of made) fs.rmSync(d, { recursive: true, force: true });
});
const run = (args, env = {}) => spawnSync(process.execPath, [path.join(tools, "reproducers.mjs"), ...args], { encoding: "utf8", maxBuffer: 1 << 26, env: { ...process.env, FULGUR: "", FAKE_FULGUR_FAIL: "", ...env } });
function printer(dir, name, env) {
  const file = path.join(dir, name);
  const sets = Object.entries(env).map(([k, v]) => `${k}='${v}'`).join(" ");
  fs.writeFileSync(file, `#!/bin/sh\n${sets} exec '${process.execPath}' '${fakeFulgur}' "$@"\n`, { mode: 0o755 });
  return file;
}

test("reproducers.mjs refuses bad printers, unknown names and folders outside fixtures/", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  const cases = [
    [["--out", path.join(out, "x")], /usage:/],
    [["--out", path.join(out, "x"), "--printer", bin], /expected LABEL=BIN/],
    [["--out", path.join(out, "x"), "--printer", `a=${bin}`, "--printer", `a=${bin}`], /same label/],
    [["--out", path.join(out, "x"), "--printer", `a=${bin}`, "no-such-page"], /no such reproducer: no-such-page/],
    [["--out", path.join(out, "x"), "--printer", `a=${bin}`, "--dir", out], /not a folder inside fixtures/],
    [["--out", path.join(tools, "..", "printers", "x"), "--printer", `a=${bin}`], /inside fixtures/],
  ];
  for (const [args, message] of cases) {
    const r = run(args);
    assert.equal(r.status, 2, `${args.join(" ")}\n${r.stdout}${r.stderr}`);
    assert.match(r.stderr, message);
  }
  assert.equal(fs.existsSync(path.join(out, "x")), false);
});

test("a reproducer is printed by Chrome and each printer, scored, and drawn side by side", (t) => {
  const require = createRequire(import.meta.url);
  let chromium;
  try {
    chromium = require("playwright").chromium.executablePath();
  } catch (e) {
    return t.skip(`playwright not available: ${e.message}`);
  }
  if (!fs.existsSync(chromium)) return t.skip("Playwright's Chromium is not installed");
  if (spawnSync("pdftoppm", ["-v"], { encoding: "utf8" }).error) return t.skip("pdftoppm (poppler-utils) is not installed");
  const out = tmp();
  const square = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "1.0.0" });
  const dead = printer(out, "other-b", { FAKE_FULGUR_VERSION: "0.1.0", FAKE_FULGUR_FAIL: "abort" });
  const dir = path.join(out, "run");
  const r = run(["--out", dir, "--printer", `square=${square}`, "--printer", `dead=${dead}`, REPRODUCER]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const text = fs.readFileSync(path.join(dir, "reproducers.json"), "utf8");
  const result = JSON.parse(text);
  assert.match(result.chromium, /^\d+\./);
  assert.deepEqual(result.printers.map((p) => [p.label, p.version]), [["square", "fulgur 1.0.0"], ["dead", "fulgur 0.1.0"]]);
  const [row] = result.rows;
  assert.deepEqual([row.name, row.gap, row.chrome.pages], [REPRODUCER, "F1", 1]);
  // The stand-in prints a black square on A4; Chrome's page is 400 × 300 px.
  assert.equal(row.printers.square.pass, false);
  assert.ok(row.printers.square.failed.includes("size"), JSON.stringify(row.printers.square));
  assert.deepEqual(row.printers.square.size, [794, 1123]);
  assert.deepEqual(row.printers.dead, { pass: false, failed: ["no output"], no_output: true });
  assert.deepEqual(row.images, [`${REPRODUCER}.png`]);
  assert.ok(fs.statSync(path.join(dir, row.images[0])).size > 1000);
  const md = fs.readFileSync(path.join(dir, "reproducers.md"), "utf8");
  assert.match(md, /\| F1 \| `F1-border-side-colour\.html` \| 1 \| \*\*FAIL\*\* · [\d.]+% \(size 794×1123.*\| \*\*FAIL\*\* \(no output\) \| \[image\]\(F1-border-side-colour\.png\) \|/);
  // How the printer died, and this machine's paths, are in nothing that gets committed.
  for (const committed of [text, md]) {
    assert.doesNotMatch(committed, /SIGABRT|killed|signal|exited/i);
    for (const where of [out, repo, os.tmpdir()]) assert.ok(!committed.includes(where), `names ${where}`);
  }
  assert.equal(readFailures(dir)[0].printer_exit.signal, "SIGABRT");
  assert.equal(fs.existsSync(path.join(dir, ".work")), false);
});

function readFailures(dir) {
  return JSON.parse(fs.readFileSync(path.join(dir, ".private", "failures.json"), "utf8"));
}
