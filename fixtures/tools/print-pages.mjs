#!/usr/bin/env node
// Real-world pages printed: each page of fixtures/real-world through the facade's
// render_page (fixtures/tools/page-check), then fulgur, compared with the committed
// Chrome references. Results as in fixtures/checkpoint-2.
//
//   node print-pages.mjs --out DIR --check BIN [--commit SHA] [--fulgur FULGUR] [--measure N]
//                        [--keep-prints] [--plain] [fixture ...]
//
// BIN is surimono-page-check; FULGUR (or $FULGUR) names fulgur if it isn't on PATH, so
// one run prints with one fulgur and printers.mjs runs this once per version. With
// --measure N each print is also timed and its memory sampled (bench/tools/measure.py,
// N runs each) and lands in print-pages.json as `print`, next to `printed`, the
// PDF's size and sha256. With --keep-prints what the printer was given stays in
// DIR/.tree (each page's print-page.html and print-floor.html) and print-pages.json
// names it as `input`, so printers.mjs can print the same thing again. With --plain
// each page is also rendered with printer-safe output off (page-check
// --no-printer-safe, the page's DOM as it is) and printed, so the two outputs stand
// side by side. DIR must be outside fixtures/. In DIR/<fixture>/:
//
//   surimono.html      what render_page gave (printer-safe, the default)
//   determinism.json   twice in one process, twice in a fresh one with another
//                      time zone and locale: all four byte-identical (with --plain,
//                      the plain output's four as well, under `plain`)
//   dom.json           the DOM the page's scripts left (render_page keeping its
//                      scripts) against Chrome's, reference/dom.html: equal or not,
//                      with each run of whitespace taken as one space and `style`
//                      moved to the end of its tag, as crates/surimono/tests/page.rs
//                      compares them
//   page/              fulgur prints surimono.html: score.json, side-by-side-1.png
//   plain/             with --plain: fulgur prints surimono-plain.html, the output
//                      with printer-safe off
//   floor/             fulgur prints Chrome's own DOM (reference/dom.html): what
//                      fulgur alone can reach on this page
//
// A printer resolves a page's relative URLs against the file it reads, so both
// are printed from a stand-in of fixtures/ in DIR, where ../../fonts and ../lib mean
// what they mean to the page. `print_fonts` in a page's meta.toml names a folder of
// .woff2 fonts to pass with --font, as fulgur doesn't load @font-face URLs. Writes DIR/print-pages.json and DIR/index.md. Exits 0
// when everything ran (a failing comparison is a result), 1 when a page's renders
// differ or the run couldn't start (pdftoppm missing, for one), 2 on usage errors.
// A print or a comparison that died is a failing score with `error`, in score.json
// and in print-pages.json, never a row without numbers and without a reason.

import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, TOOLS_DIR, listFixtures, popplerVersion } from "./lib/fixtures.mjs";
import { fulgurMeasure, fulgurPrint, fulgurVersion, pdfIdentity, useFulgur } from "./lib/fulgur.mjs";
import { passes, pct, scoresCell, verdict } from "./lib/report.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { out: { type: "string" }, check: { type: "string" }, commit: { type: "string" }, fulgur: { type: "string" }, measure: { type: "string" }, "keep-prints": { type: "boolean", default: false }, plain: { type: "boolean", default: false } },
});
const measureRuns = values.measure === undefined ? 0 : Number(values.measure);
if (!values.out || !values.check || !Number.isInteger(measureRuns) || measureRuns < 0) {
  console.error("usage: node print-pages.mjs --out DIR --check BIN [--commit SHA] [--fulgur FULGUR] [--measure N] [--keep-prints] [--plain] [fixture ...]");
  process.exit(2);
}
useFulgur(values.fulgur);
// Without the rasteriser no page can be compared: say so now, not as a score without numbers.
try {
  popplerVersion();
} catch (e) {
  console.error(`${e.message}: print-pages.mjs compares each printed page as an image`);
  process.exit(1);
}
const keep = values["keep-prints"];
const plain = values.plain;
const outRoot = path.resolve(values.out);
const inside = path.relative(FIXTURES_DIR, outRoot);
if (!inside.startsWith("..") && !path.isAbsolute(inside)) {
  console.error(`--out ${outRoot} is inside fixtures/; write results elsewhere`);
  process.exit(2);
}
const REPO = path.resolve(FIXTURES_DIR, "..");
const bin = path.resolve(values.check);
const sha256 = (data) => crypto.createHash("sha256").update(data).digest("hex");
const producer = { cli: "fixtures/tools/page-check", version: "render_page", sha256: sha256(fs.readFileSync(bin)), commit: values.commit ?? null };
const fulgur = fulgurVersion();

/** The two environments a page renders in: output must not depend on them. */
const RUNS = [
  { name: "a", TZ: "UTC", LC_ALL: "C.UTF-8" },
  { name: "b", TZ: "Asia/Tokyo", LC_ALL: "de_DE.UTF-8" },
];

/** A process of page-check; two renders on one Renderer. `name` is the files' prefix. */
function renderAll(run, dir, names, name = run.name, flags = []) {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), "surimono-pages-"));
  const r = spawnSync(bin, ["pages", "--root", REPO, "--out", dir, "--run", name, ...flags, ...names], {
    encoding: "utf8",
    cwd,
    env: { ...process.env, TZ: run.TZ, LC_ALL: run.LC_ALL, LANG: run.LC_ALL },
  });
  fs.rmSync(cwd, { recursive: true, force: true });
  if (r.status !== 0) {
    console.error(`page-check exited ${r.status ?? r.signal}: ${r.stderr}`);
    process.exit(1);
  }
}

function stamp(dir, score) {
  const stamped = { ...score, producer };
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "score.json"), `${JSON.stringify(stamped, null, 2)}\n`);
  return stamped;
}

function compare(fixture, pdf, dir) {
  fs.rmSync(path.join(dir, "score.json"), { force: true });
  const r = spawnSync(process.execPath, [path.join(TOOLS_DIR, "compare.mjs"), "--report-only", "--label", "surimono", fixture.dir, pdf, dir], { encoding: "utf8", maxBuffer: 1 << 26 });
  process.stdout.write(r.stdout);
  const file = path.join(dir, "score.json");
  if (r.status !== 0 || !fs.existsSync(file)) {
    // The reason first: a stack's first line is a file name.
    const why = r.error?.message ?? /^\w*Error: .*$/m.exec(r.stderr ?? "")?.[0] ?? `${r.stderr ?? ""}`.trim().split("\n").pop();
    return stamp(dir, { subject: fixture.name, pass: false, error: `compare.mjs exited ${r.status ?? r.signal}: ${why}\n${r.stderr ?? ""}`, pages: [] });
  }
  return stamp(dir, JSON.parse(fs.readFileSync(file, "utf8")));
}

/**
 * `html` as crates/surimono/tests/page.rs's `normal` has it: each run of ASCII
 * whitespace one space, and each tag's `style` moved to its end, since Chrome adds
 * that attribute lazily (crates/dom/tests/real_world.rs says why).
 */
function normalDom(html) {
  const squeezed = html.split(/[\t\n\f\r ]+/).filter(Boolean).join(" ");
  let out = "";
  let rest = squeezed;
  for (;;) {
    const at = rest.indexOf(' style="');
    if (at < 0) break;
    const head = rest.slice(0, at);
    const tail = rest.slice(at);
    const end = tail.slice(' style="'.length).indexOf('"');
    const close = tail.indexOf(">");
    if (end < 0 || close < 0) return out + rest;
    const attr = tail.slice(0, end + ' style=""'.length);
    out += head + tail.slice(attr.length, close) + attr;
    rest = tail.slice(close);
  }
  return out + rest;
}

/**
 * Whether the DOM render_page left (scripts kept) is Chrome's: `{ equal, differs_at }`,
 * the first byte of the two normalized texts that differs.
 */
function domCheck(ours, chrome) {
  const [a, b] = [Buffer.from(normalDom(ours)), Buffer.from(normalDom(chrome))];
  if (a.equals(b)) return { equal: true, differs_at: null };
  let at = 0;
  while (at < a.length && at < b.length && a[at] === b[at]) at++;
  return { equal: false, differs_at: at };
}

/** fulgur prints `html` (`part` is "page", "plain" or "floor") as if it lay beside the page's input.html. */
function print(tree, fixture, html, dir, part) {
  const meta = fs.readFileSync(path.join(fixture.dir, "meta.toml"), "utf8");
  const own = /^print_fonts = "(.*)"$/m.exec(meta)?.[1];
  const ownDir = own && path.join(fixture.dir, own);
  const extra = ownDir ? fs.readdirSync(ownDir).filter((f) => f.endsWith(".woff2")).sort().map((f) => path.join(ownDir, f)) : [];
  fs.mkdirSync(dir, { recursive: true });
  const standIn = path.join(tree, "real-world", fixture.name, keep ? `print-${part}.html` : "print.html");
  fs.mkdirSync(path.dirname(standIn), { recursive: true });
  fs.writeFileSync(standIn, html);
  const pdf = path.join(dir, "print.pdf");
  try {
    fulgurPrint(standIn, pdf, dir, extra);
    const score = compare(fixture, pdf, dir);
    // What was printed, and the same print again, timed and sampled; kept beside the
    // score, not in score.json.
    score.printed = pdfIdentity(pdf);
    if (keep) score.input = { html: standIn, fonts: extra };
    if (measureRuns) score.print = fulgurMeasure(standIn, pdf, dir, extra, measureRuns);
    return score;
  } catch (e) {
    return stamp(dir, { subject: fixture.name, pass: false, error: e.message, printer: e.printer, pages: [] });
  } finally {
    if (!keep) fs.rmSync(standIn, { force: true });
    fs.rmSync(pdf, { force: true });
    fs.rmSync(path.join(dir, "fonts.conf"), { force: true });
    fs.rmSync(path.join(dir, "fontconfig-cache"), { recursive: true, force: true });
  }
}

fs.mkdirSync(outRoot, { recursive: true });
const raw = path.join(outRoot, ".renders");
const tree = path.join(outRoot, ".tree");
fs.rmSync(raw, { recursive: true, force: true });
fs.rmSync(tree, { recursive: true, force: true });
fs.mkdirSync(path.join(tree, "real-world"), { recursive: true });
fs.symlinkSync(path.join(FIXTURES_DIR, "fonts"), path.join(tree, "fonts"));
fs.symlinkSync(path.join(FIXTURES_DIR, "real-world", "lib"), path.join(tree, "real-world", "lib"));

const fixtures = listFixtures("real-world", positionals);
const names = fixtures.map((f) => f.name);
for (const run of RUNS) renderAll(run, raw, names);
if (plain) for (const run of RUNS) renderAll(run, raw, names, `${run.name}-plain`, ["--no-printer-safe"]);
renderAll(RUNS[0], raw, names, "dom", ["--keep-scripts"]);

/** The four renders of one mode of a page: their files, whether all are there, their hashes. */
function renders(from, suffix) {
  const files = RUNS.flatMap((r) => [1, 2].map((n) => path.join(from, `${r.name}${suffix}-${n}.html`)));
  if (!files.every((f) => fs.existsSync(f))) return { files, complete: false };
  const hashes = files.map((f) => sha256(fs.readFileSync(f)));
  return { files, complete: true, hashes, deterministic: new Set(hashes).size === 1 };
}

const rows = [];
let differ = false;
for (const fixture of fixtures) {
  const dir = path.join(outRoot, fixture.name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  console.log(`\n== ${fixture.name}`);
  const from = path.join(raw, fixture.name);
  const reportsOf = (suffix) => RUNS.flatMap((r) => JSON.parse(fs.readFileSync(path.join(from, `${r.name}${suffix}.json`), "utf8")));
  const reports = reportsOf("");
  const safe = renders(from, "");
  const row = { fixture: fixture.name, report: reports[0], render_ms: reports[0].ms };
  if (!safe.complete) {
    row.page = stamp(path.join(dir, "page"), { subject: fixture.name, pass: false, error: reports.find((r) => r.error)?.error ?? "no output", pages: [] });
    row.deterministic = false;
  } else {
    row.deterministic = safe.deterministic;
    row.sha256 = safe.hashes[0];
    differ ||= !safe.deterministic;
    fs.copyFileSync(safe.files[0], path.join(dir, "surimono.html"));
    row.page = print(tree, fixture, fs.readFileSync(safe.files[0]), path.join(dir, "page"), "page");
  }
  const determinism = { subject: fixture.name, deterministic: row.deterministic, sha256: safe.hashes ?? [], runs: RUNS.map((r) => ({ ...r, renders: 2, process: "fresh" })), producer };
  if (plain) {
    const p = renders(from, "-plain");
    const plainReports = reportsOf("-plain");
    row.plain_sha256 = p.hashes?.[0];
    row.plain_deterministic = p.complete && p.deterministic;
    determinism.plain = { deterministic: row.plain_deterministic, sha256: p.hashes ?? [] };
    determinism.deterministic = row.deterministic = row.deterministic && row.plain_deterministic;
    differ ||= p.complete && !p.deterministic;
    row.plain = p.complete
      ? print(tree, fixture, fs.readFileSync(p.files[0]), path.join(dir, "plain"), "plain")
      : stamp(path.join(dir, "plain"), { subject: fixture.name, pass: false, error: plainReports.find((r) => r.error)?.error ?? "no output", pages: [] });
    if (p.complete) fs.copyFileSync(p.files[0], path.join(dir, "surimono-plain.html"));
  }
  if (safe.complete) fs.writeFileSync(path.join(dir, "determinism.json"), `${JSON.stringify(determinism, null, 2)}\n`);
  const kept = path.join(from, "dom-1.html");
  row.dom = fs.existsSync(kept)
    ? domCheck(fs.readFileSync(kept, "utf8"), fs.readFileSync(path.join(fixture.dir, "reference", "dom.html"), "utf8"))
    : { equal: false, differs_at: null, error: JSON.parse(fs.readFileSync(path.join(from, "dom.json"), "utf8")).find((r) => r.error)?.error ?? "no output" };
  fs.writeFileSync(path.join(dir, "dom.json"), `${JSON.stringify({ subject: fixture.name, ...row.dom, against: "reference/dom.html", producer }, null, 2)}\n`);
  // A printer's parser has scripting off, so a <noscript> would show: Chrome's DOM without
  // them. fulgur reads nothing above its input's folder, so a stylesheet goes in as text.
  const chromeDom = fs
    .readFileSync(path.join(fixture.dir, "reference", "dom.html"), "utf8")
    .replace(/<noscript\b[^]*?<\/noscript>/g, "")
    .replace(/<link rel="stylesheet" href="([^"]+)">/g, (all, href) => `<style>${fs.readFileSync(path.join(fixture.dir, href), "utf8")}</style>`);
  row.floor = print(tree, fixture, chromeDom, path.join(dir, "floor"), "floor");
  rows.push(row);
}
fs.rmSync(raw, { recursive: true, force: true });
if (!keep) fs.rmSync(tree, { recursive: true, force: true });

const image = (name, part) => (fs.existsSync(path.join(outRoot, name, part, "side-by-side-1.png")) ? `[image](${name}/${part}/side-by-side-1.png)` : "—");
const passing = (part) => rows.filter((r) => passes(r[part])).length;
const domCell = (d) => (d.equal ? "yes" : d.error ? `**no** (${String(d.error).split("\n")[0].slice(0, 100)})` : `**no** (from byte ${d.differs_at})`);
const parts = [["page", plain ? "Printer-safe" : "Printed"], ...(plain ? [["plain", "`--no-printer-safe`"]] : []), ["floor", "Floor"]];
const md = [
  "# Real-world pages against Chrome",
  "",
  `\`render_page\` (\`fixtures/tools/page-check\`${producer.commit ? `, commit ${producer.commit}` : ""}, default limits), fixture fonts only, then fulgur ${fulgur}. Chrome references: \`fixtures/real-world/*/reference/\` (Chromium 153, pinned). Generated by \`fixtures/tools/print-pages.mjs${plain ? " --plain" : ""}\`.`,
  "",
  plain
    ? `**${passing("page")} of ${rows.length} pages pass** with printer-safe output (the default), **${passing("plain")} of ${rows.length}** with \`--no-printer-safe\` (the page's DOM as it is); the floor passes on ${passing("floor")} of ${rows.length}. Byte-identical, twice in one process and twice in a fresh one under ${RUNS[1].TZ} and ${RUNS[1].LC_ALL}, both outputs: ${rows.filter((r) => r.deterministic).length} of ${rows.length}. The DOM the scripts leave equals Chrome's: ${rows.filter((r) => r.dom.equal).length} of ${rows.length}.`
    : `**${passing("page")} of ${rows.length} pages pass.** Byte-identical, twice in one process and twice in a fresh one under ${RUNS[1].TZ} and ${RUNS[1].LC_ALL}: ${rows.filter((r) => r.deterministic).length} of ${rows.length}. The DOM the scripts leave equals Chrome's: ${rows.filter((r) => r.dom.equal).length} of ${rows.length}.`,
  "",
  `The floor is fulgur printing Chrome's own DOM (\`reference/dom.html\`), so a page that fails on both sides is fulgur's. "DOM = Chrome's" is \`render_page\` keeping the page's scripts, against \`reference/dom.html\`, with each run of whitespace taken as one space and \`style\` moved to the end of its tag. Scores are shape / colour / hue. Each image is Chrome | surimono | diff.`,
  "",
  `| Page | ${parts.map(([, title]) => `${title} | Scores | Image`).join(" | ")} | DOM = Chrome's | Render | Byte-identical |`,
  `|---|${parts.map(() => "---|---|---|").join("")}---|---|---|`,
  ...rows.map((r) => `| ${r.fixture} | ${parts.map(([part]) => `${verdict(r[part])} | ${scoresCell(r[part])} | ${image(r.fixture, part)}`).join(" | ")} | ${domCell(r.dom)} | ${r.render_ms} ms | ${r.deterministic ? "yes" : "**no**"} |`),
  "",
].join("\n");
fs.writeFileSync(path.join(outRoot, "index.md"), md);
const brief = (s, pages) => ({ pass: passes(s), error: s.error, printer: s.printer, worst_window: s.worst_window, ...(pages ? { pages: s.pages?.length } : {}), printed: s.printed, input: s.input, print: s.print });
const stripped = rows.map(({ fixture, deterministic, sha256: hash, plain_sha256, render_ms, report, page, plain: p, floor, dom }) => ({ fixture, deterministic, sha256: hash, ...(plain ? { plain_sha256 } : {}), render_ms, report, page: brief(page, true), ...(plain ? { plain: brief(p, true) } : {}), floor: brief(floor, false), dom }));
fs.writeFileSync(path.join(outRoot, "print-pages.json"), `${JSON.stringify({ fulgur, producer, plain, rows: stripped }, null, 2)}\n`);
console.log(`\n${md}`);
process.exit(differ ? 1 : 0);
