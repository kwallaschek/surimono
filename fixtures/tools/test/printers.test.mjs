// The printer tooling with stand-ins for every binary (test/fake-fulgur.mjs,
// fake-page-check.mjs, fake-surimono.mjs): what printers.mjs and the --fulgur,
// --no-chrome and --measure options of checkpoint.mjs and print-pages.mjs do,
// never what a printer prints.  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const tools = path.resolve(here, "..");
const repo = path.resolve(tools, "..", "..");
const fakeSurimono = path.join(here, "fake-surimono.mjs");
const fakeCheck = path.join(here, "fake-page-check.mjs");
const fakeFulgur = path.join(here, "fake-fulgur.mjs");
const PAGE = "noscript-message";
const ISLANDS = "echarts-bar-simple-latin";

const made = [];
const tmp = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "printers-test-"));
  made.push(d);
  return d;
};
test.after(() => {
  for (const d of made) fs.rmSync(d, { recursive: true, force: true });
});
const readJson = (f) => JSON.parse(fs.readFileSync(f, "utf8"));
const node = (script, args, env = {}) =>
  spawnSync(process.execPath, [path.join(tools, script), ...args], { encoding: "utf8", maxBuffer: 1 << 26, env: { ...process.env, FULGUR: "", FAKE_SURIMONO_FROM: "", FAKE_SURIMONO_FAIL: "", FAKE_SURIMONO_LEAK: "", FAKE_FULGUR_FAIL: "", ...env } });

/** A fake fulgur as its own executable, with its version and page size fixed. */
function printer(dir, name, env) {
  const file = path.join(dir, name);
  const sets = Object.entries(env).map(([k, v]) => `${k}='${v}'`).join(" ");
  fs.writeFileSync(file, `#!/bin/sh\n${sets} exec '${process.execPath}' '${fakeFulgur}' "$@"\n`, { mode: 0o755 });
  return file;
}

test("print-pages.mjs --fulgur prints with that fulgur and --measure records each print", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "9.9.1" });
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, "--measure", "1", PAGE]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const result = readJson(path.join(out, "run", "print-pages.json"));
  assert.equal(result.fulgur, "9.9.1");
  const [row] = result.rows;
  assert.equal(row.fixture, PAGE);
  assert.equal(row.deterministic, true);
  // A black square isn't the page: the comparison ran on the fake's PDF and failed.
  assert.equal(row.page.pass, false);
  assert.equal(typeof row.page.worst_window, "number");
  for (const part of [row.page, row.floor]) {
    assert.equal(part.print.runs, 1);
    assert.ok(part.print.wall_ms.median > 0, JSON.stringify(part.print));
    assert.ok(part.print.peak_pss_mb.median > 0, JSON.stringify(part.print));
    assert.equal(part.print.memory_complete, true);
  }
  assert.match(fs.readFileSync(path.join(out, "run", "index.md"), "utf8"), /then fulgur 9\.9\.1\./);
});

test("without --measure nothing is timed, and $FULGUR still names the printer", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "9.9.2" });
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, PAGE], { FULGUR: bin });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const result = readJson(path.join(out, "run", "print-pages.json"));
  assert.equal(result.fulgur, "9.9.2");
  assert.equal(result.rows[0].page.print, undefined);
});

test("a printer that dies is a failing score that says how it ended, never a missing one", () => {
  const out = tmp();
  for (const [how, expected] of [
    ["exit", { status: 1, signal: null, timed_out: false }],
    ["abort", { status: null, signal: "SIGABRT", timed_out: false }],
  ]) {
    const bin = printer(out, `fulgur-${how}`, { FAKE_FULGUR_FAIL: how });
    const dir = path.join(out, how);
    const r = node("print-pages.mjs", ["--out", dir, "--check", fakeCheck, "--fulgur", bin, "--measure", "1", PAGE]);
    assert.equal(r.status, 0, r.stdout + r.stderr);
    const score = readJson(path.join(dir, PAGE, "page", "score.json"));
    assert.equal(score.pass, false);
    assert.deepEqual(score.printer, expected);
    assert.match(score.error, how === "exit" ? /fulgur exited 1/ : /fulgur was killed by SIGABRT/);
    assert.equal(readJson(path.join(dir, "print-pages.json")).rows[0].page.pass, false);
  }
});

test("checkpoint.mjs --no-chrome runs no gate and says it isn't a checkpoint", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "9.9.3" });
  const dir = path.join(out, "run");
  const r = node("checkpoint.mjs", ["--out", dir, "--surimono", fakeSurimono, "--no-chrome", "--fulgur", bin, ISLANDS], { FAKE_SURIMONO_FAIL: ISLANDS });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const result = readJson(path.join(dir, "checkpoint.json"));
  assert.equal(result.chrome_gate, false);
  assert.equal(result.versions.fulgur, "9.9.3");
  assert.equal(result.rows[0].islands[0].chrome, null);
  assert.equal(result.rows[0].islands[0].fulgur.pass, false);
  assert.equal(fs.existsSync(path.join(dir, ISLANDS, "island-0-chrome")), false);
  const md = fs.readFileSync(path.join(dir, "index.md"), "utf8");
  assert.match(md, /\*\*Not a checkpoint: run with `--no-chrome`/);
  assert.match(md, /All comparisons: 0 of 2 pass/);
  assert.match(md, /\| 0 \| not run \| — \| \*\*FAIL\*\*/);
});

test("printers.mjs refuses bad printers, unknown fixtures and an --out inside fixtures/", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  const base = ["--surimono", fakeSurimono, "--check", fakeCheck];
  const cases = [
    [["--out", path.join(out, "x"), ...base], /usage:/],
    [["--out", path.join(out, "x"), ...base, "--printer", bin], /expected LABEL=FULGUR/],
    [["--out", path.join(out, "x"), ...base, "--printer", `a b=${bin}`], /expected LABEL=FULGUR/],
    [["--out", path.join(out, "x"), ...base, "--printer", `a=${bin}`, "--printer", `a=${bin}`], /same label/],
    [["--out", path.join(out, "x"), ...base, "--printer", `a=${bin}`, "no-such-page"], /no such fixture: no-such-page/],
    [["--out", path.join(out, "x"), ...base, "--printer", `a=${bin}`, "--runs", "0"], /usage:/],
    [["--out", path.join(tools, "..", "printers", "x"), ...base, "--printer", `a=${bin}`], /inside fixtures/],
  ];
  for (const [args, message] of cases) {
    const r = node("printers.mjs", args);
    assert.equal(r.status, 2, `${args.join(" ")}\n${r.stdout}${r.stderr}`);
    assert.match(r.stderr, message);
  }
  assert.equal(fs.existsSync(path.join(out, "x")), false);
  const gone = node("printers.mjs", ["--out", path.join(out, "x"), ...base, "--printer", `a=${path.join(out, "no-such-fulgur")}`]);
  assert.equal(gone.status, 1, gone.stdout + gone.stderr);
  assert.match(gone.stderr, /--version failed/);
});

test("printers.mjs puts two printers side by side and draws only the cells that differ", () => {
  const out = tmp();
  const a = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "1.0.0" });
  const same = printer(out, "fulgur-b", { FAKE_FULGUR_VERSION: "1.0.1" });
  const letter = printer(out, "fulgur-c", { FAKE_FULGUR_VERSION: "2.0.0", FAKE_FULGUR_PAGE: "letter" });
  const dir = path.join(out, "run");
  const r = node("printers.mjs", ["--out", dir, "--surimono", fakeSurimono, "--check", fakeCheck, "--commit", "0123abc", "--runs", "1", "--printer", `one=${a}`, "--printer", `two=${same}`, "--printer", `three=${letter}`, PAGE]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const text = fs.readFileSync(path.join(dir, "printers.json"), "utf8");
  const result = JSON.parse(text);
  assert.deepEqual(result.printers.map((p) => [p.label, p.fulgur]), [["one", "1.0.0"], ["two", "1.0.1"], ["three", "2.0.0"]]);
  assert.deepEqual(result.rows.map((row) => row.key), [`real-world/${PAGE}/page`, `real-world/${PAGE}/floor`]);
  for (const row of result.rows) {
    // Same pixels from one and two; three prints another page size, which fails "size" too.
    assert.equal(row.differs, true);
    assert.equal(row.pixels_identical, false);
    assert.deepEqual(row.printers.one, { ...row.printers.two, print: row.printers.one.print });
    assert.equal(row.printers.one.pass, false);
    assert.ok(!row.printers.one.failed.includes("size"));
    assert.ok(row.printers.three.failed.includes("size"), JSON.stringify(row.printers.three));
    assert.ok(row.printers.three.print.wall_ms.median > 0);
    assert.deepEqual(row.images, [`${PAGE}/${row.what}.png`]);
    assert.ok(fs.statSync(path.join(dir, row.images[0])).size > 1000);
  }
  assert.equal(result.totals.one.prints, 2);
  // Nothing committed names this machine.
  const md = fs.readFileSync(path.join(dir, "index.md"), "utf8");
  for (const where of [out, repo, os.tmpdir()]) {
    assert.ok(!text.includes(where), `printers.json names ${where}`);
    assert.ok(!md.includes(where), `index.md names ${where}`);
  }
  assert.match(md, /fulgur 1\.0\.0 and fulgur 1\.0\.1 and fulgur 2\.0\.0/);
  assert.match(md, /## Island fixture pages\n\nNone in this run\./);
  assert.match(md, /\| noscript-message \| surimono's page \| \*\*FAIL\*\* · [\d.]+% .*\| \*\*FAIL\*\* · [\d.]+% \(size 816×1056, Chrome's 794×1123; .*\| no \| \[image\]\(noscript-message\/page\.png\) \|/);

  // Two printers that print the same: no cell differs, no image.
  const dir2 = path.join(out, "run2");
  const r2 = node("printers.mjs", ["--out", dir2, "--surimono", fakeSurimono, "--check", fakeCheck, "--runs", "1", "--printer", `one=${a}`, "--printer", `two=${same}`, PAGE]);
  assert.equal(r2.status, 0, r2.stdout + r2.stderr);
  for (const row of readJson(path.join(dir2, "printers.json")).rows) {
    assert.equal(row.differs, false);
    assert.equal(row.pixels_identical, true);
    assert.equal(row.images, undefined);
  }
  assert.equal(fs.existsSync(path.join(dir2, PAGE)), false);
});
