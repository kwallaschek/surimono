#!/usr/bin/env node
// What each printer prints of the pages here, and of surimono's printer-safe output
// for them: the evidence behind docs/COMPAT.md's print-media rows that
// crates/surimono/tests/data/print-media/ doesn't already measure.
//
//   node fixtures/printers/print-media/measure.mjs --surimono CLI --fulgur FULGUR_0_40 --raikiri RAIKIRI_PRINT
//
// For each page <p>.html this writes <p>.output.html (`CLI render <p>.html --scripts`
// with the fixture fonts and no resource: `not-given.css` is never given) and, in
// measured.json, the labels (the words of the page's visible <p> elements) that each
// printer prints for the page and for the output:
//
// - "chromium": the pinned Chromium as the fixtures' reference pipeline prints
//   (page.pdf, preferCSSPageSize, printBackground, print media, Playwright's default
//   paper, Letter); "chromium_a4": the same with format A4; "chromium_screen": the
//   same printed with emulated `screen` media (what Grover's emulate_media "screen"
//   does); "screen": the labels shown on screen (no printing: the <p> elements whose
//   computed display isn't none, with screen media);
// - "fulgur" and "raikiri": fulgur 0.40.0 and fulgur-raikiri 0.41.0 (through
//   fixtures/tools/raikiri-print), with the fixture fonts only, as lib/fulgur.mjs runs them.
//
// Run it holding the fleet's build lock. The labels are read with pdftotext.

import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFileSync, spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";
import { FIXTURES_DIR, LOCALE, ORIGIN, TIMEZONE, launchChromium, routeOffline } from "../../tools/lib/fixtures.mjs";
import { fulgurPrint } from "../../tools/lib/fulgur.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: { surimono: { type: "string" }, fulgur: { type: "string" }, raikiri: { type: "string" } } });
if (!values.surimono || !values.fulgur || !values.raikiri) {
  console.error("usage: node measure.mjs --surimono CLI --fulgur FULGUR_0_40 --raikiri RAIKIRI_PRINT");
  process.exit(2);
}
const FONTS = fs.readdirSync(path.join(FIXTURES_DIR, "fonts")).filter((f) => /\.(ttf|otf)$/.test(f)).sort().map((f) => path.join(FIXTURES_DIR, "fonts", f));
const labels = (pdf) => execFileSync("pdftotext", [pdf, "-"], { encoding: "utf8" }).split(/\s+/).filter(Boolean).sort();
const version = (bin) => spawnSync(bin, ["--version"], { encoding: "utf8" }).stdout.trim().split("\n")[0];
const work = fs.mkdtempSync(path.join(os.tmpdir(), "print-media-"));
const pages = fs.readdirSync(HERE).filter((f) => f.endsWith(".html") && !f.endsWith(".output.html")).map((f) => f.slice(0, -5)).sort();

const browser = await launchChromium();
async function chromium(page, html, how) {
  const context = await browser.newContext({ timezoneId: TIMEZONE, locale: LOCALE, deviceScaleFactor: 1 });
  try {
    const at = `/printers/print-media/${html === undefined ? `${page}.html` : `__${page}.output.html`}`;
    await routeOffline(context, html === undefined ? new Map() : new Map([[at, html]]));
    const tab = await context.newPage();
    await tab.goto(`${ORIGIN}${at}`, { waitUntil: "load" });
    await tab.evaluate(() => document.fonts.ready.then(() => 0));
    if (how === "screen") {
      await tab.emulateMedia({ media: "screen" });
      return (await tab.evaluate(() => [...document.querySelectorAll("p")].filter((p) => getComputedStyle(p).display !== "none").map((p) => p.textContent))).sort();
    }
    await tab.emulateMedia({ media: how === "chromium_screen" ? "screen" : "print" });
    const pdf = path.join(work, `${page}-${how}-${html === undefined ? "page" : "output"}.pdf`);
    await tab.pdf({ path: pdf, preferCSSPageSize: true, printBackground: true, ...(how === "chromium_a4" ? { format: "A4" } : {}) });
    return labels(pdf);
  } finally {
    await context.close();
  }
}

function printer(bin, page, file) {
  const pdf = path.join(work, `${page}-${path.basename(bin)}-${path.basename(file)}.pdf`);
  fulgurPrint(file, pdf, work, [], bin);
  return labels(pdf);
}

const measured = { chromium: browser.version(), fulgur: version(values.fulgur), raikiri: version(values.raikiri), surimono: version(values.surimono), pages: {} };
for (const page of pages) {
  const input = path.join(HERE, `${page}.html`);
  const output = path.join(HERE, `${page}.output.html`);
  execFileSync(values.surimono, ["render", input, "-o", output, "--scripts", ...FONTS.flatMap((f) => ["--font", f])]);
  const html = fs.readFileSync(output, "utf8");
  // Printed from here, so the output's relative URLs resolve as the page's do.
  const outFile = path.join(HERE, `__${page}.output.html`);
  fs.writeFileSync(outFile, html);
  const m = { output_is_page: html === fs.readFileSync(input, "utf8") };
  try {
    for (const how of ["chromium", "chromium_a4", "chromium_screen", "screen"]) m[how] = { page: await chromium(page, undefined, how), output: await chromium(page, html, how) };
    m.fulgur = { page: printer(values.fulgur, page, input), output: printer(values.fulgur, page, outFile) };
    m.raikiri = { page: printer(values.raikiri, page, input), output: printer(values.raikiri, page, outFile) };
  } finally {
    fs.rmSync(outFile);
  }
  measured.pages[page] = m;
  console.log(page, JSON.stringify(m));
}
await browser.close();
fs.rmSync(work, { recursive: true, force: true });
fs.writeFileSync(path.join(HERE, "measured.json"), `${JSON.stringify(measured, null, 1)}\n`);
