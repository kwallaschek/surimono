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
// text. The output and its print are kept in DIR/<case>/.
//
// Loads (meta.toml `loads`, default ["http"]): how Chromium gets the page and the output.
// "http" serves them as text/html; charset=utf-8; "file" opens them from file:// URLs (a
// copy of the case folder beside a link to fixtures/fonts, in DIR); "http-no-charset"
// serves text/html with no charset. The reference is the first load's print. Every load
// checks the page's decoding (document.characterSet) against `page_charset.<load>`, and
// requires the output to decode, print and read as the page does, except in a load the
// case lists under `mojibake`: there the page has no charset <meta>, isn't decoded as
// UTF-8, and nothing promises how the output decodes (DECISIONS 2026-11-16), so the
// output's decoding and print are recorded, not compared.
//
// What the printer-safe pass did (meta.toml `printer_safe`), against the plain output
// (`--no-printer-safe`): "dom" nothing; "sheet" it only added <style> elements (print
// media left as written: the gate left the page alone); "media" more than that (print
// media resolved: media preludes or attributes rewritten, a <style> silenced). A run that
// finds otherwise says so in its log and in neutrality.json (`printer_safe_note`) and
// still passes: the field records what the case exercises, and a narrower or wider gate
// shows there first.
//
// Writes DIR/neutrality.json. Exits 0 when every case holds, 1 when a reference
// changed or an output prints differently, 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
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

/** The loads a case may name (see the header). */
export const LOADS = ["http", "file", "http-no-charset"];
/** What a printer-safe output may differ from the plain one by (see the header). */
export const PRINTER_SAFE = ["dom", "sheet", "media"];

/**
 * A case's meta.toml beyond `resources`: `printer_safe`, `loads` (default ["http"]),
 * `page_charset.<load>` (required for every load when `loads` is given) and `mojibake`
 * (loads where the page is mojibake by design; each must have a page_charset other than
 * UTF-8). Throws on anything it can't use.
 */
export function metaOf(caseDir) {
  const file = path.join(caseDir, "meta.toml");
  const text = fs.readFileSync(file, "utf8");
  const list = (key) => {
    const m = new RegExp(`^${key}\\s*=\\s*\\[([^\\]]*)\\]\\s*$`, "m").exec(text);
    return m ? [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]) : null;
  };
  const printerSafe = /^printer_safe\s*=\s*"([^"]*)"\s*$/m.exec(text)?.[1];
  if (!PRINTER_SAFE.includes(printerSafe)) throw new Error(`${file}: printer_safe must be one of ${PRINTER_SAFE.join(", ")}`);
  const declared = list("loads");
  const loads = declared ?? ["http"];
  if (!loads.length || loads.some((l) => !LOADS.includes(l)) || new Set(loads).size !== loads.length) throw new Error(`${file}: loads must be distinct names among ${LOADS.join(", ")}`);
  const pageCharset = Object.fromEntries([...text.matchAll(/^page_charset\.([\w-]+)\s*=\s*"([^"]+)"\s*$/gm)].map((m) => [m[1], m[2]]));
  for (const l of Object.keys(pageCharset)) if (!loads.includes(l)) throw new Error(`${file}: page_charset.${l} names a load the case doesn't list`);
  if (declared) for (const l of loads) if (!pageCharset[l]) throw new Error(`${file}: no page_charset.${l} (each load the case lists says how Chromium decodes the page there)`);
  const mojibake = list("mojibake") ?? [];
  for (const l of mojibake) {
    if (!loads.includes(l)) throw new Error(`${file}: mojibake names ${l}, which isn't one of its loads`);
    if (!pageCharset[l] || pageCharset[l] === "UTF-8") throw new Error(`${file}: mojibake load ${l} needs a page_charset other than UTF-8`);
  }
  return { printerSafe, loads, pageCharset, mojibake };
}

/**
 * What the printer-safe output changed against the plain one: "dom" (nothing),
 * "sheet" (only added <style> elements) or "media" (anything more). A <style> element
 * of the output is taken out when it occurs there more often than in the plain output,
 * so a rewritten or silenced <style> counts as more.
 */
export function printerSafeChange(plain, output) {
  if (plain === output) return "dom";
  const count = (s, x) => s.split(x).length - 1;
  let rest = output;
  for (const e of new Set(output.match(/<style\b[^>]*>[\s\S]*?<\/style>/gi) ?? [])) {
    for (let extra = count(rest, e) - count(plain, e); extra > 0; extra--) rest = rest.replace(e, "");
  }
  return rest === plain ? "sheet" : "media";
}

/**
 * One load's verdict on the output: `ok`, and `result` "same", "differs" (with what) or
 * "not compared" (a mojibake load). `got` holds the page's and the output's charset, and
 * whether their pixels and text are the same.
 */
export function judgeLoad(load, meta, got) {
  const wanted = meta.pageCharset[load];
  if (wanted && got.page_charset !== wanted) return { ok: false, result: `the page decodes as ${got.page_charset}, meta.toml says ${wanted}` };
  if (!("output_charset" in got)) return { ok: true, result: "page only" };
  if (meta.mojibake.includes(load)) return { ok: true, result: "not compared" };
  const off = [
    got.output_charset !== got.page_charset && `decodes as ${got.output_charset}`,
    !got.pixels_same && "pixels",
    !got.text_same && "text",
  ].filter(Boolean);
  return off.length ? { ok: false, result: `differs: ${off.join(", ")}` } : { ok: true, result: "same" };
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

/**
 * Chromium prints the case's input.html, or `html` (a Buffer: the output's bytes) in its
 * place, as the reference pipeline prints, through `load`; returns document.characterSet.
 * A file load reads from `tree` (made by fileTree).
 */
async function printInChrome(browser, c, pdf, html, load = "http", tree = null) {
  const context = await browser.newContext({ timezoneId: TIMEZONE, locale: LOCALE, deviceScaleFactor: 1 });
  try {
    const name = html === undefined ? "input.html" : "__output.html";
    const at = `/neutrality/${c.name}/${name}`;
    await routeOffline(context, html === undefined || load === "file" ? new Map() : new Map([[at, html]]), { charset: load !== "http-no-charset", files: tree });
    let url = `${ORIGIN}${at}`;
    if (load === "file") {
      const file = path.join(tree, "neutrality", c.name, name);
      if (html !== undefined) fs.writeFileSync(file, html);
      url = pathToFileURL(file).href;
    }
    const page = await context.newPage();
    const response = await page.goto(url, { waitUntil: "load" });
    if (!response || !response.ok()) throw new Error(`${c.name}: could not load ${url}`);
    await page.evaluate(async () => {
      await Promise.all([...document.fonts].map((f) => f.load().catch(() => null)));
      await document.fonts.ready;
    });
    const charset = await page.evaluate(() => document.characterSet);
    await page.emulateMedia({ media: "print" });
    await page.pdf({ path: pdf, preferCSSPageSize: true, printBackground: true });
    return charset;
  } finally {
    await context.close();
  }
}

/** A copy of the case folder at DIR/<case>/tree/neutrality/<case>/, beside a link to fixtures/fonts, for file loads. */
function fileTree(c, dir) {
  const tree = path.join(dir, "tree");
  fs.cpSync(c.dir, path.join(tree, "neutrality", c.name), { recursive: true, filter: (src) => !src.startsWith(path.join(c.dir, "reference")) });
  fs.symlinkSync(path.join(FIXTURES_DIR, "fonts"), path.join(tree, "fonts"));
  return tree;
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
        const meta = metaOf(c.dir);
        row.loads = {};
        const tree = meta.loads.includes("file") ? fileTree(c, dir) : null;
        // The page in each load; the first is the one the reference holds.
        const page = {};
        for (const load of meta.loads) {
          const ld = path.join(dir, load);
          const charset = await printInChrome(browser, c, path.join(ld, "chrome.pdf"), undefined, load, tree);
          page[load] = { pages: rasterise(path.join(ld, "chrome.pdf"), ld, "chrome"), text: pdftotext(path.join(ld, "chrome.pdf")) };
          row.loads[load] = { page_charset: charset };
        }
        const { pages, text } = page[meta.loads[0]];
        const refDir = path.join(c.dir, "reference");
        if (values.update) {
          fs.rmSync(refDir, { recursive: true, force: true });
          fs.mkdirSync(refDir, { recursive: true });
          const lines = ["# Chromium's print of input.html. Generated by fixtures/tools/neutrality.mjs --update; never edit by hand.", `chromium = ${JSON.stringify(chromium)}`, `playwright = ${JSON.stringify(pinnedVersions().playwright)}`, `poppler = ${JSON.stringify(poppler)}`, `dpi = ${DPI}`, `load = ${JSON.stringify(meta.loads[0])}`, `pages = ${pages.length}`, ""];
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
          // What the printer-safe pass wrote: the output against the page's DOM as it is
          // (--no-printer-safe), so a case that tests nothing shows.
          render(cli, c, path.join(dir, "plain.html"), ["--no-printer-safe"]);
          const html = fs.readFileSync(output);
          row.printer_safe = printerSafeChange(fs.readFileSync(path.join(dir, "plain.html"), "utf8"), html.toString("utf8"));
          if (row.printer_safe !== meta.printerSafe) row.printer_safe_note = `meta.toml says ${meta.printerSafe}`;
          for (const load of meta.loads) {
            const ld = path.join(dir, load);
            const got = row.loads[load];
            got.output_charset = await printInChrome(browser, c, path.join(ld, "output.pdf"), html, load, tree);
            const printed = rasterise(path.join(ld, "output.pdf"), ld, "output");
            got.pixels_same = samePixels(page[load].pages.map(readPng), printed.map(readPng));
            got.text_same = pdftotext(path.join(ld, "output.pdf")) === page[load].text;
            if (!got.pixels_same) writePng(path.join(ld, "output-vs-page.png"), sideBySide([readPng(page[load].pages[0]), readPng(printed[0])]));
          }
        }
        for (const load of meta.loads) Object.assign(row.loads[load], judgeLoad(load, meta, row.loads[load]));
      } catch (e) {
        row.error = e.message;
      }
      const loads = Object.entries(row.loads ?? {});
      row.ok = !row.error && ["identical", "reproduced", "updated"].includes(row.reference) && !row.reference_text && loads.every(([, l]) => l.ok);
      const said = loads.map(([name, l]) => `${name} ${l.page_charset}${"output_charset" in l ? ` ${l.result}` : l.ok ? "" : ` (${l.result})`}`).join(", ");
      console.log(`${row.ok ? "ok  " : "FAIL"} ${c.name}: reference ${row.reference ?? "-"}${row.reference_text ? `, text ${row.reference_text}` : ""}${cli ? `; printer-safe ${row.printer_safe ?? "-"}${row.printer_safe_note ? ` (NOTE: ${row.printer_safe_note})` : ""}` : ""}; ${said}${row.error ? `; ${row.error}` : ""}`);
      rows.push(row);
    }
  } finally {
    await browser.close();
  }
  const failed = rows.filter((r) => !r.ok);
  fs.writeFileSync(path.join(outRoot, "neutrality.json"), `${JSON.stringify({ chromium, poppler, surimono: cli ? spawnSync(cli, ["--version"], { encoding: "utf8" }).stdout.trim() : null, outputs_checked: Boolean(cli), rows }, null, 2)}\n`);
  console.log(`${rows.length - failed.length} of ${rows.length} neutrality cases hold${cli ? "" : " (references only: no --surimono, so no output was checked)"}`);
  if (cli) {
    const by = (k) => rows.filter((r) => r.printer_safe === k).length;
    const notes = rows.filter((r) => r.printer_safe_note);
    console.log(`printer-safe output: ${by("media")} resolve print media, ${by("sheet")} only add the sheet, ${by("dom")} are the DOM`);
    if (notes.length) console.log(`NOTE: ${notes.length} case(s) differ from their meta.toml printer_safe (update the field and say why in the pull request): ${notes.map((r) => `${r.case} is ${r.printer_safe}, ${r.printer_safe_note}`).join("; ")}`);
  }
  if (failed.length) process.exit(1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) await main();
