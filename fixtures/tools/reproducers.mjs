#!/usr/bin/env node
// Small pages that each show one printing gap, printed by Chrome and by every
// printer named, and each printer's page scored against Chrome's by the metric
// and pass rule of fixtures/README.md.
//
//   node reproducers.mjs --out DIR --printer LABEL=BIN [--printer LABEL=BIN ...]
//                        [--dir fixtures/printers/reproducers] [name ...]
//
// A reproducer is an .html file in --dir (inside fixtures/, which is all Chrome is
// served) whose name starts with the gap it shows, as docs/upstream names them:
// F1-border-side-colour.html. It loads its fonts from ../../fonts with @font-face,
// as a fixture does; the printers get the same fonts with --font. Chrome is the
// pinned Chromium of the references, and it prints each page anew on every run:
// nothing here is a committed expectation, and what Chrome printed is the first
// panel of every image. BIN takes `fulgur render`'s command line (printers.mjs).
// DIR must be outside fixtures/. Writes:
//
//   DIR/reproducers.json   per reproducer and printer: verdict and scores
//   DIR/reproducers.md     rows: reproducers; columns: the printers
//   DIR/<name>.png         Chrome | printer 1 | printer 2 | …, one per page
//
// A printer that gives no PDF, or one nothing can read, is a cell that says "no
// output"; why is in DIR/.private/failures.json, which isn't for committing.
// Exits 0 when everything ran, 1 when Chrome couldn't print a page or the run
// couldn't start, 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, ORIGIN, launchChromium, pinnedVersions, popplerVersion, rasterise, renderFixtureInChrome } from "./lib/fixtures.mjs";
import { PASS_COLOUR_WINDOW, PASS_HUE_WINDOW, PASS_WORST_WINDOW, comparePdf } from "./lib/compare.mjs";
import { fulgurPrint } from "./lib/fulgur.mjs";
import { pct } from "./lib/report.mjs";
import { readPng, sideBySide, writePng } from "./lib/png.mjs";
import { PNG } from "pngjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { out: { type: "string" }, dir: { type: "string", default: path.join(FIXTURES_DIR, "printers", "reproducers") }, printer: { type: "string", multiple: true } },
});
const usage = (why) => {
  if (why) console.error(why);
  console.error("usage: node reproducers.mjs --out DIR --printer LABEL=BIN [--printer LABEL=BIN ...] [--dir fixtures/printers/reproducers] [name ...]");
  process.exit(2);
};
if (!values.out || !values.printer?.length) usage();
const printers = values.printer.map((p) => {
  const at = p.indexOf("=");
  if (at < 1 || !/^[\w.+-]+$/.test(p.slice(0, at))) usage(`--printer ${p}: expected LABEL=BIN, the label of letters, digits, ".", "+", "-" and "_"`);
  return { label: p.slice(0, at), bin: path.resolve(p.slice(at + 1)) };
});
if (new Set(printers.map((p) => p.label)).size !== printers.length) usage("two printers have the same label");
const outRoot = path.resolve(values.out);
const dir = path.resolve(values.dir);
const within = (parent, child) => {
  const rel = path.relative(parent, child);
  return !rel.startsWith("..") && !path.isAbsolute(rel);
};
if (within(FIXTURES_DIR, outRoot)) {
  console.error(`--out ${outRoot} is inside fixtures/; write results elsewhere`);
  process.exit(2);
}
if (!within(FIXTURES_DIR, dir) || !fs.existsSync(dir)) usage(`--dir ${dir}: not a folder inside fixtures/`);
const all = fs.readdirSync(dir).filter((f) => f.endsWith(".html")).map((f) => f.slice(0, -5)).sort((a, b) => a.localeCompare(b, "en", { numeric: true }));
const missing = positionals.filter((n) => !all.includes(n));
if (missing.length) usage(`no such reproducer: ${missing.join(", ")}`);
const names = positionals.length ? all.filter((n) => positionals.includes(n)) : all;
if (!names.length) usage(`no .html file in ${dir}`);

const REPO = path.resolve(FIXTURES_DIR, "..");
const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
try {
  popplerVersion();
} catch (e) {
  console.error(`${e.message}: reproducers.mjs compares each printed page as an image`);
  process.exit(1);
}
for (const p of printers) {
  const v = spawnSync(p.bin, ["--version"], { encoding: "utf8" });
  if (v.status !== 0) {
    console.error(`${p.bin} --version failed: ${v.error?.message ?? v.stderr}`);
    process.exit(1);
  }
  p.version = v.stdout.trim().split("\n")[0];
  p.sha256 = sha256(p.bin);
}

fs.mkdirSync(outRoot, { recursive: true });
const work = path.join(outRoot, ".work");
fs.rmSync(work, { recursive: true, force: true });
const failures = [];
const rows = [];
const browser = await launchChromium();
const chromium = browser.version();
try {
  for (const name of names) {
    console.log(`\n== ${name}`);
    const html = path.join(dir, `${name}.html`);
    const mine = path.join(work, name);
    fs.mkdirSync(mine, { recursive: true });
    const url = `${ORIGIN}/${path.relative(FIXTURES_DIR, html).split(path.sep).join("/")}`;
    // As a real-world page is printed: scripts settled, fonts loaded, and an error if
    // any text was drawn in a system font.
    await renderFixtureInChrome(browser, { name, kind: "printers", url }, path.join(mine, "chrome.pdf"));
    const chrome = rasterise(path.join(mine, "chrome.pdf"), mine, "chrome");
    const row = { name, gap: /^[A-Z]+\d+/.exec(name)?.[0] ?? null, chrome: { pages: chrome.length, sha256: chrome.map(sha256) }, printers: {} };
    const panels = [chrome];
    for (const p of printers) {
      const out = path.join(mine, p.label);
      const pdf = path.join(out, "print.pdf");
      fs.mkdirSync(out, { recursive: true });
      try {
        fulgurPrint(html, pdf, out, [], p.bin);
        const s = comparePdf({ expectedPngs: chrome, pdf, outDir: out, label: "printed", subject: `${name} (${p.label})` });
        const failed = [...new Set(s.pages.flatMap((page) => page.failed ?? ["page count"]))];
        row.printers[p.label] = { pass: s.pass === true, failed, shape: s.worst_window, colour: s.worst_colour_window, hue: s.worst_hue_window, pages: s.page_count.printed, size: s.pages[0]?.actual_size ?? null };
        panels.push(fs.readdirSync(out).filter((f) => /^printed-\d+\.png$/.test(f)).sort((a, b) => parseInt(a.slice(8), 10) - parseInt(b.slice(8), 10)).map((f) => path.join(out, f)));
        console.log(`${s.pass ? "pass" : "FAIL"} ${p.label}: shape ${pct(s.worst_window)}, colour ${pct(s.worst_colour_window)}, hue ${pct(s.worst_hue_window)}, ${s.page_count.printed} page(s)`);
      } catch (e) {
        failures.push({ name, printer: p.label, error: e.message, printer_exit: e.printer ?? null });
        row.printers[p.label] = { pass: false, failed: ["no output"], no_output: true };
        panels.push([]);
        console.log(`no output from ${p.label}`);
      }
    }
    // One image per page: Chrome, then each printer (a white page of Chrome's size where one has none).
    row.images = [];
    const pages = Math.max(...panels.map((list) => list.length));
    for (let i = 0; i < pages; i++) {
      const images = panels.map((list) => (list[i] ? readPng(list[i]) : null));
      const like = images.find(Boolean);
      const file = `${name}${pages > 1 ? `-${i + 1}` : ""}.png`;
      writePng(path.join(outRoot, file), sideBySide(images.map((img) => img ?? blank(like.width, like.height))));
      row.images.push(file);
    }
    rows.push(row);
  }
} finally {
  await browser.close();
}
fs.rmSync(work, { recursive: true, force: true });

/** A white image, where a printer has no such page. */
function blank(width, height) {
  const img = new PNG({ width, height });
  img.data.fill(255);
  return img;
}

const labels = printers.map((p) => p.label);
const cell = (c) => {
  if (c.no_output) return "**FAIL** (no output)";
  if (c.pass) return `pass · ${pct(c.shape)}`;
  const detail = { shape: null, colour: `colour ${pct(c.colour)}`, hue: `hue ${pct(c.hue)}`, size: `size ${c.size?.join("×")}`, "page count": `${c.pages} pages` };
  const other = c.failed.map((f) => (f in detail ? detail[f] : f)).filter(Boolean);
  return `**FAIL** · ${pct(c.shape)}${other.length ? ` (${other.join("; ")})` : ""}`;
};
const md = [
  "# Reproducers against Chrome",
  "",
  `One small page per printing gap, printed by Chrome (Chromium ${chromium}, the references' build) and by ${printers.map((p) => p.version).join(", ").replace(/, ([^,]*)$/, " and $1")}, fixture fonts only. Each printer's page is scored against Chrome's by the metric and pass rule of \`fixtures/README.md\`: shape ≤ ${pct(PASS_WORST_WINDOW)}, colour ≤ ${pct(PASS_COLOUR_WINDOW)}, hue ≤ ${pct(PASS_HUE_WINDOW)}, same page size and count. A pass says the printer's page is Chrome's within that rule; a gap whose effect is smaller than the rule sees can pass, so each image is there to be looked at: Chrome | ${labels.join(" | ")}. Generated by \`fixtures/tools/reproducers.mjs\`.`,
  "",
  `| Gap | Reproducer | Chrome's pages | ${labels.join(" | ")} | Image |`,
  `|---|---|---|${labels.map(() => "---").join("|")}|---|`,
  ...rows.map((r) => `| ${r.gap ?? ""} | \`${r.name}.html\` | ${r.chrome.pages} | ${labels.map((l) => cell(r.printers[l])).join(" | ")} | ${r.images.map((f) => `[image](${f})`).join(" ")} |`),
  "",
  `| Printer | Reproducers that pass |`,
  "|---|---|",
  ...printers.map((p) => `| ${p.version} | ${rows.filter((r) => r.printers[p.label].pass).length} of ${rows.length} |`),
  "",
].join("\n");
const json = {
  generated_by: "fixtures/tools/reproducers.mjs",
  chromium,
  playwright: pinnedVersions().playwright,
  poppler: popplerVersion(),
  pass_rule: { shape: PASS_WORST_WINDOW, colour: PASS_COLOUR_WINDOW, hue: PASS_HUE_WINDOW },
  printers: printers.map(({ label, version, sha256: hash }) => ({ label, version, sha256: hash })),
  rows,
};
const relabel = (text) => text.split(`${outRoot}/`).join("reproducers-out/").split(outRoot).join("reproducers-out").split(`${REPO}/`).join("").split(REPO).join(".");
fs.rmSync(path.join(outRoot, ".private"), { recursive: true, force: true });
if (failures.length) {
  fs.mkdirSync(path.join(outRoot, ".private"), { recursive: true });
  fs.writeFileSync(path.join(outRoot, ".private", "failures.json"), `${JSON.stringify(failures, null, 2)}\n`);
  console.error(`${failures.length} cell(s) without output; the reasons are in ${path.join(outRoot, ".private", "failures.json")}, which isn't for committing`);
}
fs.writeFileSync(path.join(outRoot, "reproducers.json"), relabel(`${JSON.stringify(json, null, 2)}\n`));
fs.writeFileSync(path.join(outRoot, "reproducers.md"), relabel(md));
console.log(`\n${relabel(md)}`);
