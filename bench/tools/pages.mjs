#!/usr/bin/env node
// Builds the two derived pages of the benchmark from bench/report/report.html:
//
//   <work>/report-grover.html       the report as a Grover user ships it: the same
//                                   markup plus <script> tags for the pinned
//                                   ECharts, Vega and Vega-Lite builds, which draw
//                                   every island client-side in Chrome and then set
//                                   <html data-islands="done">.
//   <work>/report-prerendered.html  the report with each island marker replaced by
//                                   the SVG Chrome drew for it (the input of the
//                                   "fulgur only" reference point).
//
//   node bench/tools/pages.mjs <work-dir>
//
// Prints one JSON line: {"grover": path, "prerendered": path, "islands": n}.
// The island drawing code is fixtures/tools/lib/islands-browser.js, the same
// contract the Chrome references use.

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { prepassHtml, TOOLS_DIR } from "../../fixtures/tools/lib/fixtures.mjs";

const require = createRequire(path.join(TOOLS_DIR, "package.json"));
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const REPORT = path.join(ROOT, "bench", "report", "report.html");

const work = path.resolve(process.argv[2] || "");
if (!process.argv[2]) {
  console.error("usage: node bench/tools/pages.mjs <work-dir>");
  process.exit(2);
}
fs.mkdirSync(work, { recursive: true });

const source = fs.readFileSync(REPORT, "utf8");
const nm = path.join(TOOLS_DIR, "node_modules");
const scripts = [
  path.join(nm, "echarts", "dist", "echarts.min.js"),
  path.join(nm, "vega", "build", "vega.min.js"),
  path.join(nm, "vega-lite", "build", "vega-lite.min.js"),
  path.join(TOOLS_DIR, "lib", "islands-browser.js"),
];
for (const s of scripts) if (!fs.existsSync(s)) throw new Error(`${s} missing: run npm ci in fixtures/tools`);

const base = `<base href="${pathToFileURL(path.dirname(REPORT)).href}/">`;
const tags = scripts.map((s) => `<script src="${pathToFileURL(s).href}"></script>`).join("\n");
const boot = `<script>
window.__surimonoRenderIslands().then(
  (r) => { window.__benchIslands = r.islands; document.documentElement.setAttribute("data-islands", "done"); },
  (e) => { document.documentElement.setAttribute("data-islands", "error"); setTimeout(() => { throw e; }); });
</script>`;
if (!source.includes("<head>") || !source.includes("</body>")) throw new Error("report.html has no <head> or </body>");
const grover = source.replace("<head>", `<head>\n  ${base}`).replace("</body>", `${tags}\n${boot}\n</body>`);
const groverPath = path.join(work, "report-grover.html");
fs.writeFileSync(groverPath, grover);

// Chrome draws the islands once more to get their SVGs for the prerendered page.
const { chromium } = require("playwright");
const browser = await chromium.launch({ channel: "chromium" });
let svgs;
try {
  const context = await browser.newContext({ timezoneId: "UTC", locale: "en-US", deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(pathToFileURL(groverPath).href, { waitUntil: "load" });
  await page.waitForSelector("html[data-islands]", { timeout: 60000 });
  const state = await page.getAttribute("html", "data-islands");
  if (state !== "done") throw new Error(`islands failed in Chrome: ${errors.join("; ")}`);
  svgs = await page.evaluate(() => window.__benchIslands.map((i) => i.svg));
} finally {
  await browser.close();
}
const prerenderedPath = path.join(work, "report-prerendered.html");
fs.writeFileSync(prerenderedPath, prepassHtml(source, svgs).replace("<head>", `<head>\n  ${base}`));
console.log(JSON.stringify({ grover: groverPath, prerendered: prerenderedPath, islands: svgs.length }));
