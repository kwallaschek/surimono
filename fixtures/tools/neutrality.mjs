#!/usr/bin/env node
// Neutrality cases: pages where surimono's printer-safe output must print in Chromium
// exactly as the page itself does (DECISIONS 2026-11-14 and 2026-11-16: "Chromium
// prints the output as it prints the page, whatever the page's CSS").
//
//   node neutrality.mjs --out DIR [--update] [--surimono CLI] [case ...]
//
// Each case is fixtures/neutrality/<case>/ (fixtures/README.md, "Neutrality cases"):
// input.html, the files it links, meta.toml and reference/ (Chromium's print of
// input.html). For each case this has the pinned Chromium print input.html, as the
// reference pipeline prints (offline from the fixtures origin, print media,
// preferCSSPageSize, printBackground), rasterises it at 96 dpi and:
//
// - check mode (default): compares it with the committed reference/chrome-N.png by
//   reference.mjs's rule (identical, or only anti-aliasing differs: reproduced) and its
//   text (pdftotext) with reference/chrome.txt;
// - --update: writes reference/ (only for a new case or a deliberate Chromium
//   upgrade, said so in the pull request; never to make surimono pass).
//
// With --surimono CLI it also renders the case (`CLI render input.html --scripts`,
// the fixture fonts, a --resource for each file meta.toml's `resources` names; the
// default printer-safe output), has Chromium print that output from the same folder,
// and requires it to be pixel for pixel the page's print of this run and the same
// text. The output and its print are kept in DIR/<case>/, and `printer_safe_changed`
// says whether the output differs from the page's DOM as it is (`--no-printer-safe`).
//
// Writes DIR/neutrality.json. Exits 0 when every case holds, 1 when a reference
// changed or an output prints differently, 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { DPI, FIXTURES_DIR, LOCALE, ORIGIN, TIMEZONE, launchChromium, pinnedVersions, popplerVersion, rasterise, routeOffline } from "./lib/fixtures.mjs";
import { diff, pixelsIdentical, readPng, sideBySide, writePng } from "./lib/png.mjs";

export const CASES_DIR = path.join(FIXTURES_DIR, "neutrality");

/** The cases: folders of fixtures/neutrality with an input.html, sorted. */
export function listCases(only = []) {
  const names = fs.existsSync(CASES_DIR)
    ? fs
        .readdirSync(CASES_DIR, { withFileTypes: true })
        .filter((d) => d.isDirectory() && fs.existsSync(path.join(CASES_DIR, d.name, "input.html")))
        .map((d) => d.name)
        .sort()
    : [];
  const missing = only.filter((n) => !names.includes(n));
  if (missing.length) throw new Error(`no such neutrality case: ${missing.join(", ")}`);
  return (only.length ? names.filter((n) => only.includes(n)) : names).map((name) => ({ name, dir: path.join(CASES_DIR, name) }));
}

/** meta.toml's `resources = ["a.css", …]`: the files surimono is given (others it isn't). */
export function resourcesOf(caseDir) {
  const file = path.join(caseDir, "meta.toml");
  if (!fs.existsSync(file)) throw new Error(`${file}: missing`);
  const m = /^resources\s*=\s*\[([^\]]*)\]\s*$/m.exec(fs.readFileSync(file, "utf8"));
  if (!m) throw new Error(`${file}: no resources = [...] line (write resources = [] when surimono is given nothing)`);
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** One reference page against a fresh one: identical, reproduced (anti-aliasing only) or changed. */
export function judgePage(expected, actual) {
  if (pixelsIdentical(expected, actual)) return { result: "identical" };
  const { score, image } = diff(expected, actual);
  return { result: score.size_match && score.diff_pixels === 0 ? "reproduced" : "changed", image, score };
}

/** Whether two prints are the same pixels, page for page. */
export function samePixels(a, b) {
  return a.length === b.length && a.every((p, i) => pixelsIdentical(p, b[i]));
}

const pdftotext = (pdf) => execFileSync("pdftotext", ["-layout", pdf, "-"], { encoding: "utf8" });
const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** Chromium prints `html` (or the case's input.html) as the reference pipeline prints. */
async function printInChrome(browser, c, pdf, html) {
  const context = await browser.newContext({ timezoneId: TIMEZONE, locale: LOCALE, deviceScaleFactor: 1 });
  try {
    const at = `/neutrality/${c.name}/${html === undefined ? "input.html" : "__output.html"}`;
    await routeOffline(context, html === undefined ? new Map() : new Map([[at, html]]));
    const page = await context.newPage();
    const response = await page.goto(`${ORIGIN}${at}`, { waitUntil: "load" });
    if (!response || !response.ok()) throw new Error(`${c.name}: could not load ${at}`);
    await page.evaluate(async () => {
      await Promise.all([...document.fonts].map((f) => f.load().catch(() => null)));
      await document.fonts.ready;
    });
    await page.emulateMedia({ media: "print" });
    await page.pdf({ path: pdf, preferCSSPageSize: true, printBackground: true });
  } finally {
    await context.close();
  }
}

function render(cli, c, out, extra = []) {
  const fonts = fs
    .readdirSync(path.join(FIXTURES_DIR, "fonts"))
    .filter((f) => /\.(ttf|otf)$/i.test(f))
    .sort()
    .flatMap((f) => ["--font", path.join(FIXTURES_DIR, "fonts", f)]);
  const resources = resourcesOf(c.dir).flatMap((r) => ["--resource", `${r}=${path.join(c.dir, r)}`]);
  const r = spawnSync(cli, ["render", path.join(c.dir, "input.html"), "-o", out, "--scripts", ...extra, ...fonts, ...resources], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`surimono exited ${r.status ?? r.signal}: ${`${r.stderr}${r.stdout}`.trim().split("\n").slice(0, 10).join("\n")}`);
}

async function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: { out: { type: "string" }, update: { type: "boolean", default: false }, surimono: { type: "string" } },
  });
  if (!values.out) {
    console.error("usage: node neutrality.mjs --out DIR [--update] [--surimono CLI] [case ...]");
    process.exit(2);
  }
  let cases;
  try {
    cases = listCases(positionals);
  } catch (e) {
    console.error(e.message);
    process.exit(2);
  }
  const outRoot = path.resolve(values.out);
  const inside = path.relative(FIXTURES_DIR, outRoot);
  if (!inside.startsWith("..") && !path.isAbsolute(inside)) {
    console.error(`--out ${outRoot} is inside fixtures/; write results elsewhere`);
    process.exit(2);
  }
  const poppler = popplerVersion();
  const cli = values.surimono ? path.resolve(values.surimono) : null;
  const browser = await launchChromium();
  const chromium = browser.version();
  const rows = [];
  try {
    for (const c of cases) {
      const dir = path.join(outRoot, c.name);
      fs.rmSync(dir, { recursive: true, force: true });
      fs.mkdirSync(dir, { recursive: true });
      const row = { case: c.name };
      try {
        await printInChrome(browser, c, path.join(dir, "chrome.pdf"));
        const pages = rasterise(path.join(dir, "chrome.pdf"), dir, "chrome");
        const text = pdftotext(path.join(dir, "chrome.pdf"));
        const refDir = path.join(c.dir, "reference");
        if (values.update) {
          fs.rmSync(refDir, { recursive: true, force: true });
          fs.mkdirSync(refDir, { recursive: true });
          const lines = ["# Chromium's print of input.html. Generated by fixtures/tools/neutrality.mjs --update; never edit by hand.", `chromium = ${JSON.stringify(chromium)}`, `playwright = ${JSON.stringify(pinnedVersions().playwright)}`, `poppler = ${JSON.stringify(poppler)}`, `dpi = ${DPI}`, `pages = ${pages.length}`, ""];
          for (const p of pages) {
            const dest = path.join(refDir, path.basename(p));
            fs.copyFileSync(p, dest);
            const png = readPng(dest);
            lines.push("[[page]]", `file = ${JSON.stringify(path.basename(p))}`, `width = ${png.width}`, `height = ${png.height}`, `sha256 = ${JSON.stringify(sha256(dest))}`, "");
          }
          fs.writeFileSync(path.join(refDir, "chrome.txt"), text);
          fs.writeFileSync(path.join(refDir, "reference.toml"), lines.join("\n"));
          row.reference = "updated";
        } else {
          const expected = fs.existsSync(refDir) ? fs.readdirSync(refDir).filter((f) => /^chrome-\d+\.png$/.test(f)).sort((a, b) => parseInt(a.slice(7), 10) - parseInt(b.slice(7), 10)) : [];
          if (!expected.length) row.reference = "missing";
          else if (expected.length !== pages.length) row.reference = "changed";
          else {
            const judged = pages.map((p, i) => judgePage(readPng(path.join(refDir, expected[i])), readPng(p)));
            judged.forEach((j, i) => j.image && j.result === "changed" && writePng(path.join(dir, `check-${i + 1}.png`), sideBySide([readPng(path.join(refDir, expected[i])), readPng(pages[i]), j.image])));
            row.reference = judged.some((j) => j.result === "changed") ? "changed" : judged.some((j) => j.result === "reproduced") ? "reproduced" : "identical";
          }
          if (row.reference !== "missing" && fs.readFileSync(path.join(refDir, "chrome.txt"), "utf8") !== text) row.reference_text = "changed";
        }
        if (cli) {
          const output = path.join(dir, "output.html");
          render(cli, c, output);
          // Whether the printer-safe pass wrote anything: the output against the page's DOM
          // as it is (--no-printer-safe), so a case that tests nothing shows.
          render(cli, c, path.join(dir, "plain.html"), ["--no-printer-safe"]);
          const html = fs.readFileSync(output, "utf8");
          row.printer_safe_changed = html !== fs.readFileSync(path.join(dir, "plain.html"), "utf8");
          await printInChrome(browser, c, path.join(dir, "output.pdf"), html);
          const printed = rasterise(path.join(dir, "output.pdf"), dir, "output");
          row.output_pixels = samePixels(pages.map(readPng), printed.map(readPng)) ? "same" : "differ";
          row.output_text = pdftotext(path.join(dir, "output.pdf")) === text ? "same" : "differ";
          if (row.output_pixels === "differ") writePng(path.join(dir, "output-vs-page.png"), sideBySide([readPng(pages[0]), readPng(printed[0])]));
        }
      } catch (e) {
        row.error = e.message;
      }
      row.ok = !row.error && ["identical", "reproduced", "updated"].includes(row.reference) && !row.reference_text && (!cli || (row.output_pixels === "same" && row.output_text === "same"));
      console.log(`${row.ok ? "ok  " : "FAIL"} ${c.name}: reference ${row.reference ?? "-"}${row.reference_text ? `, text ${row.reference_text}` : ""}${cli ? `; printer-safe output ${row.printer_safe_changed ? "differs from the DOM" : "is the DOM"}, pixels ${row.output_pixels ?? "-"}, text ${row.output_text ?? "-"}` : ""}${row.error ? `; ${row.error}` : ""}`);
      rows.push(row);
    }
  } finally {
    await browser.close();
  }
  const failed = rows.filter((r) => !r.ok);
  fs.writeFileSync(path.join(outRoot, "neutrality.json"), `${JSON.stringify({ chromium, poppler, surimono: cli ? spawnSync(cli, ["--version"], { encoding: "utf8" }).stdout.trim() : null, outputs_checked: Boolean(cli), rows }, null, 2)}\n`);
  console.log(`${rows.length - failed.length} of ${rows.length} neutrality cases hold${cli ? "" : " (references only: no --surimono, so no output was checked)"}`);
  if (failed.length) process.exit(1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) await main();
