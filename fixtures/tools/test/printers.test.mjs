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

// The tools rasterise with poppler's pdftoppm, which a GitHub runner doesn't have when
// these tests run. They test the tools, not a rasteriser, so `pdftoppm` is a stand-in
// (test/fake-pdftoppm.mjs) found first on PATH, like every other binary here.
const standIns = tmp();
fs.writeFileSync(path.join(standIns, "pdftoppm"), `#!/bin/sh\nexec '${process.execPath}' '${path.join(here, "fake-pdftoppm.mjs")}' "$@"\n`, { mode: 0o755 });
const node = (script, args, env = {}) =>
  spawnSync(process.execPath, [path.join(tools, script), ...args], { encoding: "utf8", maxBuffer: 1 << 26, env: { ...process.env, PATH: `${standIns}${path.delimiter}${process.env.PATH}`, FULGUR: "", FULGUR_TIMEOUT_MS: "", FAKE_SURIMONO_FROM: "", FAKE_SURIMONO_FAIL: "", FAKE_SURIMONO_LEAK: "", FAKE_FULGUR_FAIL: "", FAKE_PAGE_CHECK_PLAIN: "", FAKE_PAGE_CHECK_DOM: "", ...env } });

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
  // If it didn't run at all, its reason is the message.
  assert.equal(row.page.error, undefined, `the comparison died: ${row.page.error}`);
  assert.equal(row.floor.error, undefined, `the comparison died: ${row.floor.error}`);
  assert.equal(row.page.pass, false);
  assert.equal(typeof row.page.worst_window, "number");
  for (const part of [row.page, row.floor]) {
    assert.equal(part.print.runs, 1);
    // The single runs are kept, so the medians can be recomputed.
    assert.deepEqual(part.print.wall_ms_runs.length, 1);
    assert.ok(Math.abs(part.print.wall_ms_runs[0] - part.print.wall_ms.median) <= 0.051, JSON.stringify(part.print));
    assert.deepEqual(part.print.peak_pss_mb_runs, [part.print.peak_pss_mb.median]);
    assert.equal(part.input, undefined, "no --keep-prints, no input kept");
    assert.ok(part.print.wall_ms.median > 0, JSON.stringify(part.print));
    assert.ok(part.print.peak_pss_mb.median > 0, JSON.stringify(part.print));
    assert.equal(part.print.memory_complete, true);
  }
  assert.match(fs.readFileSync(path.join(out, "run", "index.md"), "utf8"), /then fulgur 9\.9\.1\./);
  assert.equal(fs.existsSync(path.join(out, "run", ".tree")), false);
});

test("--keep-prints leaves what the printer was given and names it", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, "--keep-prints", PAGE]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const [row] = readJson(path.join(out, "run", "print-pages.json")).rows;
  assert.equal(row.page.input.html, path.join(out, "run", ".tree", "real-world", PAGE, "print-page.html"));
  assert.equal(row.floor.input.html, path.join(out, "run", ".tree", "real-world", PAGE, "print-floor.html"));
  assert.deepEqual(row.page.input.fonts, []);
  // The page is fake-page-check's copy of Chrome's DOM; the floor is that DOM without <noscript>.
  const dom = fs.readFileSync(path.join(repo, "fixtures", "real-world", PAGE, "reference", "dom.html"), "utf8");
  assert.equal(fs.readFileSync(row.page.input.html, "utf8"), dom);
  assert.doesNotMatch(fs.readFileSync(row.floor.input.html, "utf8"), /<noscript/);
  assert.match(dom, /<noscript/);
});

test("--plain prints the output with printer-safe off beside the default one", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  const plainHtml = "<!doctype html><html><head></head><body><p>plain</p></body></html>";
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, "--keep-prints", "--plain", PAGE], { FAKE_PAGE_CHECK_PLAIN: plainHtml });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const result = readJson(path.join(out, "run", "print-pages.json"));
  assert.equal(result.plain, true);
  const [row] = result.rows;
  // Each output is printed from its own renders: the plain one is page-check's --no-printer-safe output.
  assert.equal(fs.readFileSync(row.plain.input.html, "utf8"), plainHtml);
  assert.equal(fs.readFileSync(path.join(out, "run", PAGE, "surimono-plain.html"), "utf8"), plainHtml);
  assert.notEqual(row.plain_sha256, row.sha256);
  assert.equal(row.plain.pass, false);
  assert.equal(row.plain.error, undefined, `the comparison died: ${row.plain.error}`);
  assert.equal(readJson(path.join(out, "run", PAGE, "plain", "score.json")).subject, PAGE);
  // Four renders of each, all byte-identical.
  const determinism = readJson(path.join(out, "run", PAGE, "determinism.json"));
  assert.equal(determinism.deterministic, true);
  assert.equal(determinism.sha256.length, 4);
  assert.deepEqual(determinism.plain, { deterministic: true, sha256: Array(4).fill(row.plain_sha256) });
  const index = fs.readFileSync(path.join(out, "run", "index.md"), "utf8");
  assert.match(index, /\*\*0 of 1 pages pass\*\* with printer-safe output \(the default\), \*\*0 of 1\*\* with `--no-printer-safe`/);
  assert.match(index, /\| Page \| Printer-safe \| Scores \| Image \| `--no-printer-safe` \| Scores \| Image \| Floor \| Scores \| Image \| DOM = Chrome's \| Render \| Byte-identical \|/);
  assert.match(index, new RegExp(`\\(${PAGE}/plain/side-by-side-1\\.png\\)`));
});

test("without --plain there is no plain output, and every run checks the DOM against Chrome's", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  // A page whose DOM has a `style` attribute.
  const page = "alpine-invoice";
  const dom = fs.readFileSync(path.join(repo, "fixtures", "real-world", page, "reference", "dom.html"), "utf8");
  const run = (name, env) => {
    const r = node("print-pages.mjs", ["--out", path.join(out, name), "--check", fakeCheck, "--fulgur", bin, page], env);
    assert.equal(r.status, 0, r.stdout + r.stderr);
    return { result: readJson(path.join(out, name, "print-pages.json")), index: fs.readFileSync(path.join(out, name, "index.md"), "utf8"), file: readJson(path.join(out, name, page, "dom.json")) };
  };
  const same = run("same", {});
  assert.equal(same.result.plain, false);
  assert.equal(same.result.rows[0].plain, undefined);
  assert.equal(fs.existsSync(path.join(out, "same", page, "plain")), false);
  assert.doesNotMatch(same.index, /no-printer-safe/);
  assert.deepEqual(same.result.rows[0].dom, { equal: true, differs_at: null });
  assert.equal(same.file.equal, true);
  assert.match(same.file.against, /^reference\/dom\.html without <noscript>/);
  assert.match(same.index, /The DOM the scripts leave equals Chrome's: 1 of 1\./);
  // How long a run of whitespace is and where `style` sits in a tag don't count, as in crates/surimono/tests/page.rs.
  const styled = dom.replace(/<(\w+)([^>]*?) style="([^"]*)"([^>]*)>/, '<$1 style="$3"$2$4>');
  assert.notEqual(styled, dom, "the fixture has a style attribute to move");
  const loose = run("loose", { FAKE_PAGE_CHECK_DOM: styled.replace(/ /g, " \n\t ") });
  assert.deepEqual(loose.result.rows[0].dom, { equal: true, differs_at: null });
  // Anything else does, and the cell says from where.
  const at = dom.indexOf("<body") + 2;
  const other = run("other", { FAKE_PAGE_CHECK_DOM: `${dom.slice(0, at)}X${dom.slice(at + 1)}` });
  const { equal, differs_at: from } = other.result.rows[0].dom;
  assert.equal(equal, false);
  assert.equal(typeof from, "number");
  assert.match(other.index, new RegExp(`\\| \\*\\*no\\*\\* \\(from byte ${from}\\) \\|`));
  assert.match(other.index, /The DOM the scripts leave equals Chrome's: 0 of 1\./);
});

test("a printer that never finishes is killed and reported as timed out", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-hang", { FAKE_FULGUR_FAIL: "hang" });
  const started = Date.now();
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, PAGE], { FULGUR_TIMEOUT_MS: "700" });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.ok(Date.now() - started < 60_000, "the 120 s default wasn't used");
  for (const part of ["page", "floor"]) {
    const score = readJson(path.join(out, "run", PAGE, part, "score.json"));
    assert.equal(score.pass, false);
    assert.deepEqual(score.printer, { status: null, signal: "SIGKILL", timed_out: true });
    assert.match(score.error, /fulgur didn't finish in 0\.7 s/);
  }
  assert.match(fs.readFileSync(path.join(out, "run", "index.md"), "utf8"), /\*\*0 of 1 pages pass\.\*\*/);
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

test("without a rasteriser the tools stop at once and say what is missing", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  // A PATH with nothing on it: no pdftoppm, real or stand-in.
  const empty = path.join(out, "empty-path");
  fs.mkdirSync(empty);
  const pages = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, PAGE], { PATH: empty });
  assert.equal(pages.status, 1, pages.stdout + pages.stderr);
  assert.match(pages.stderr, /pdftoppm not found \(install poppler-utils\): print-pages\.mjs compares/);
  assert.equal(fs.existsSync(path.join(out, "run", "print-pages.json")), false, "no results without a comparison");
  const matrix = node("printers.mjs", ["--out", path.join(out, "run2"), "--surimono", fakeSurimono, "--check", fakeCheck, "--printer", `a=${bin}`, PAGE], { PATH: empty });
  assert.equal(matrix.status, 1, matrix.stdout + matrix.stderr);
  assert.match(matrix.stderr, /pdftoppm not found \(install poppler-utils\): printers\.mjs compares/);
  assert.equal(fs.existsSync(path.join(out, "run2", "printers.json")), false);
});

test("a comparison that dies is a failing score with its reason, in the score and in the results", () => {
  const out = tmp();
  const bin = printer(out, "fulgur-a", {});
  // A rasteriser that answers -v and then fails on every page.
  const broken = path.join(out, "broken");
  fs.mkdirSync(broken);
  fs.writeFileSync(path.join(broken, "pdftoppm"), `#!/bin/sh\n[ "$1" = -v ] && { echo "pdftoppm version 0.0.0 (broken)" >&2; exit 0; }\necho "Syntax Error: no pages here" >&2\nexit 1\n`, { mode: 0o755 });
  const r = node("print-pages.mjs", ["--out", path.join(out, "run"), "--check", fakeCheck, "--fulgur", bin, PAGE], { PATH: `${broken}${path.delimiter}${process.env.PATH}` });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const [row] = readJson(path.join(out, "run", "print-pages.json")).rows;
  for (const part of [row.page, row.floor]) {
    assert.equal(part.pass, false);
    assert.match(part.error, /^compare\.mjs exited 1: Error: /);
    assert.equal(part.worst_window, undefined);
  }
  assert.match(readJson(path.join(out, "run", PAGE, "page", "score.json")).error, /^compare\.mjs exited 1: Error: /);
  assert.match(fs.readFileSync(path.join(out, "run", "index.md"), "utf8"), /\*\*FAIL\*\* \(render error: compare\.mjs exited 1: Error: /);
});

test("a printer that dies is a cell that says \"no output\" and nothing more", () => {
  const out = tmp();
  const good = printer(out, "fulgur-a", { FAKE_FULGUR_VERSION: "1.0.0" });
  const dead = printer(out, "other-b", { FAKE_FULGUR_VERSION: "0.1.0", FAKE_FULGUR_FAIL: "abort" });
  const dir = path.join(out, "run");
  const r = node("printers.mjs", ["--out", dir, "--surimono", fakeSurimono, "--check", fakeCheck, "--runs", "1", "--printer", `good=${good}`, "--printer", `dead=${dead}`, PAGE]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const text = fs.readFileSync(path.join(dir, "printers.json"), "utf8");
  const md = fs.readFileSync(path.join(dir, "index.md"), "utf8");
  for (const row of JSON.parse(text).rows) {
    assert.deepEqual({ pass: row.printers.dead.pass, no_output: row.printers.dead.no_output, print: row.printers.dead.print }, { pass: false, no_output: true, print: { not_measured: true } });
    assert.equal(row.differs, true);
    // The printer that printed is still scored and timed.
    assert.equal(typeof row.printers.good.shape, "number");
    assert.ok(row.printers.good.print.wall_ms.median > 0);
  }
  assert.match(md, /\| noscript-message \| surimono's page \| \*\*FAIL\*\* · [\d.]+% .*\| \*\*FAIL\*\* \(no output\) \|/);
  // How it died is nowhere in what gets committed, and all of it in .private.
  for (const committed of [text, md]) assert.doesNotMatch(committed, /SIGABRT|killed|signal|exited|error/i);
  const failures = readJson(path.join(dir, ".private", "failures.json"));
  assert.equal(failures.length, 2);
  assert.deepEqual(failures.map((f) => [f.printer, f.printer_exit.signal]), [["dead", "SIGABRT"], ["dead", "SIGABRT"]]);
  assert.match(failures[0].error, /killed by SIGABRT/);
  assert.match(r.stderr, /2 cell\(s\) without output/);
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
  const r = node("printers.mjs", ["--out", dir, "--surimono", fakeSurimono, "--check", fakeCheck, "--commit", "0123abc", "--runs", "3", "--printer", `one=${a}`, "--printer", `two=${same}`, "--printer", `three=${letter}`, PAGE]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const text = fs.readFileSync(path.join(dir, "printers.json"), "utf8");
  const result = JSON.parse(text);
  assert.deepEqual(result.printers.map((p) => [p.label, p.version]), [["one", "fulgur 1.0.0"], ["two", "fulgur 1.0.1"], ["three", "fulgur 2.0.0"]]);
  assert.deepEqual(result.rows.map((row) => row.key), [`real-world/${PAGE}/page`, `real-world/${PAGE}/floor`]);
  for (const row of result.rows) {
    for (const [label, cell] of Object.entries(row.printers)) assert.equal(cell.no_output, undefined, `${row.key}, ${label}: no output: ${fs.existsSync(path.join(dir, ".private", "failures.json")) ? fs.readFileSync(path.join(dir, ".private", "failures.json"), "utf8") : "?"}`);
    // Same pixels from one and two; three prints another page size, which fails "size" too.
    assert.equal(row.differs, true);
    assert.equal(row.pixels_identical, false);
    assert.deepEqual({ ...row.printers.one, print: null }, { ...row.printers.two, print: null });
    assert.match(row.printers.one.pdf.sha256, /^[0-9a-f]{64}$/);
    assert.ok(row.printers.one.pdf.bytes > 300);
    assert.equal(row.pdf_identical, false);
    assert.equal(row.printers.one.pass, false);
    assert.ok(!row.printers.one.failed.includes("size"));
    assert.ok(row.printers.three.failed.includes("size"), JSON.stringify(row.printers.three));
    for (const label of ["one", "two", "three"]) {
      const p = row.printers[label].print;
      assert.equal(p.wall_ms_runs.length, 3);
      assert.equal(p.peak_pss_mb_runs.length, 3);
      // The statistic is the middle run, rounded to 0.1.
      assert.ok(Math.abs(p.wall_ms.median - [...p.wall_ms_runs].sort((a, b) => a - b)[1]) <= 0.051, JSON.stringify(p));
      assert.ok(p.wall_ms.median > 0);
    }
    assert.deepEqual(row.images, [`${PAGE}/${row.what}.png`]);
    assert.ok(fs.statSync(path.join(dir, row.images[0])).size > 1000);
  }
  assert.equal(result.totals.one.prints, 2);
  // The printers take turns: who starts moves on every round, and on the next page.
  assert.deepEqual(result.rows[0].print_order, { warmup_rounds: 1, timed_first: ["one", "two", "three"], sampled_first: ["one", "two", "three"] });
  assert.deepEqual(result.rows[1].print_order.timed_first, ["two", "three", "one"]);
  assert.equal(result.measurement.runs, 3);
  // Each other printer against the first, from the runs in the file.
  for (const label of ["two", "three"]) {
    for (const field of ["wall_ms", "peak_pss_mb"]) {
      const c = result.against_first[label][field];
      assert.equal(c.pages, 2);
      assert.equal(c.pages_lower + c.pages_equal + c.pages_higher, 2);
      const diffs = result.rows.map((row) => Math.round((row.printers[label].print[field].median - row.printers.one.print[field].median) * 10) / 10);
      assert.equal(c.per_page_difference.min, Math.min(...diffs));
      assert.equal(c.per_page_difference.max, Math.max(...diffs));
      assert.ok(c.pages_with_every_run_lower + c.pages_with_every_run_higher <= 2);
    }
  }
  assert.equal(result.against_first.one, undefined);
  // Nothing of the measuring pass is left behind.
  assert.equal(fs.existsSync(path.join(dir, ".measure")), false);
  // Nothing committed names this machine.
  const md = fs.readFileSync(path.join(dir, "index.md"), "utf8");
  for (const where of [out, repo, os.tmpdir()]) {
    assert.ok(!text.includes(where), `printers.json names ${where}`);
    assert.ok(!md.includes(where), `index.md names ${where}`);
  }
  assert.match(md, /printed by fulgur 1\.0\.0, fulgur 1\.0\.1 and fulgur 2\.0\.0, fixture fonts only/);
  assert.match(md, /The printers take turns on each page/);
  assert.match(md, /## Time and memory against one\n/);
  assert.match(md, /\| two \| Print time \| [\d.]+ ms \| [\d.]+ ms \| [+−]?[\d.]+ ms /);
  assert.match(md, /\| three \| Peak PSS \| [\d.]+ MB /);
  assert.match(md, /## Island fixture pages\n\nNone in this run\./);
  assert.match(md, /\| noscript-message \| surimono's page \| \*\*FAIL\*\* · [\d.]+% .*\| \*\*FAIL\*\* · [\d.]+% \(size 816×1056, Chrome's 794×1123; .*\| no \| no \| \[image\]\(noscript-message\/page\.png\) \|/);

  // Two printers that print the same: no cell differs, no image.
  const dir2 = path.join(out, "run2");
  const r2 = node("printers.mjs", ["--out", dir2, "--surimono", fakeSurimono, "--check", fakeCheck, "--runs", "1", "--printer", `one=${a}`, "--printer", `two=${same}`, PAGE]);
  assert.equal(r2.status, 0, r2.stdout + r2.stderr);
  for (const row of readJson(path.join(dir2, "printers.json")).rows) {
    assert.equal(row.differs, false);
    assert.equal(row.pixels_identical, true);
    assert.equal(row.pdf_identical, true);
    assert.equal(row.images, undefined);
  }
  assert.equal(fs.existsSync(path.join(dir2, PAGE)), false);
});
