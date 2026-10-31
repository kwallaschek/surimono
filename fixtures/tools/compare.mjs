#!/usr/bin/env node
// Compares surimono output with a fixture's committed Chrome reference.
//
// Page mode: a PDF printed by surimono + fulgur (or any engine) against the
// full-page references:
//   node compare.mjs [--label NAME] [--report-only] <fixture> <pdf> <outdir>
//
// Island mode: one island's SVG, printed alone at the island's size (the same
// wrapper page the island references use), against reference/island-<N>.png:
//   node compare.mjs --island N [--printer chrome|fulgur] [--label NAME] [--report-only] <fixture> <svg> <outdir>
// With --printer chrome (default) both sides are drawn by Chrome, so the score
// measures surimono's SVG alone. With --printer fulgur it also includes how
// fulgur prints SVG.
//
// <fixture> is a fixture name (searched in fixtures/*/) or its folder.
// Rasterises like the references (pdftoppm, 96 dpi) and writes to <outdir>:
// <label>-<n>.png, side-by-side-<n>.png (Chrome | label | diff; red differs,
// yellow is anti-aliasing only) and score.json. Exits 1 when a page or island
// fails the threshold (unless --report-only), 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, islandPageHtml, launchChromium, listFixtures, printIslandInChrome } from "./lib/fixtures.mjs";
import { comparePdf, printResult, referenceFiles } from "./lib/compare.mjs";
import { fulgurPrint } from "./lib/fulgur.mjs";
import { compareTexts } from "./lib/svgtext.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    label: { type: "string", default: "surimono" },
    island: { type: "string" },
    printer: { type: "string", default: "chrome" },
    "report-only": { type: "boolean", default: false },
  },
});
const usage = () => {
  console.error("usage: node compare.mjs [--label NAME] [--report-only] <fixture> <pdf> <outdir>\n       node compare.mjs --island N [--printer chrome|fulgur] [--label NAME] [--report-only] <fixture> <svg> <outdir>");
  process.exit(2);
};
if (positionals.length !== 3 || !["chrome", "fulgur"].includes(values.printer)) usage();

function resolveFixture(arg) {
  if (fs.existsSync(path.join(arg, "input.html"))) {
    const dir = path.resolve(arg);
    return { name: path.basename(dir), kind: path.basename(path.dirname(dir)), dir };
  }
  for (const kind of fs.readdirSync(FIXTURES_DIR)) {
    if (fs.existsSync(path.join(FIXTURES_DIR, kind, arg, "input.html"))) return listFixtures(kind, [arg])[0];
  }
  throw new Error(`no fixture named or at "${arg}"`);
}

const fixture = resolveFixture(positionals[0]);
const input = path.resolve(positionals[1]);
const out = path.resolve(positionals[2]);
fs.mkdirSync(out, { recursive: true });
const refs = referenceFiles(fixture.dir);

let result;
if (values.island === undefined) {
  if (!refs.pages.length) {
    console.error(`${fixture.name}: no Chrome reference committed (run fixtures/tools/reference.sh --update)`);
    process.exit(2);
  }
  result = comparePdf({ expectedPngs: refs.pages, pdf: input, outDir: out, label: values.label, subject: fixture.name });
} else {
  const index = Number(values.island);
  const expected = refs.islands[index];
  if (!Number.isInteger(index) || !expected) {
    console.error(`${fixture.name}: no committed reference/island-${values.island}.png`);
    process.exit(2);
  }
  const svg = fs.readFileSync(input, "utf8");
  const size = /<svg\b[^>]*?\swidth="([\d.]+)(?:px)?"[^>]*?\sheight="([\d.]+)(?:px)?"/.exec(svg) || /<svg\b[^>]*?\sheight="([\d.]+)(?:px)?"[^>]*?\swidth="([\d.]+)(?:px)?"/.exec(svg);
  const meta = fs.readFileSync(path.join(fixture.dir, "reference", "reference.toml"), "utf8");
  const islandMeta = [...meta.matchAll(/\[\[island\]\]\nindex = (\d+)\nfile = "[^"]+"\nwidth = (\d+)\nheight = (\d+)/g)].find((m) => Number(m[1]) === index);
  if (!islandMeta) throw new Error(`reference.toml has no [[island]] ${index}`);
  const [width, height] = [Number(islandMeta[2]), Number(islandMeta[3])];
  if (!size) console.log(`note: the SVG root has no width/height attributes; printing it in a ${width}x${height} box`);
  const pdf = path.join(out, `${values.label}.pdf`);
  if (values.printer === "chrome") {
    const browser = await launchChromium();
    try {
      await printIslandInChrome(browser, fixture, index, svg, width, height, pdf);
    } finally {
      await browser.close();
    }
  } else {
    // Served from the fixture's folder in Chrome; for fulgur the fonts come from --font.
    const html = path.join(out, `${values.label}-island-${index}.html`);
    fs.writeFileSync(html, islandPageHtml(svg, width, height));
    fulgurPrint(html, pdf, out);
  }
  result = comparePdf({ expectedPngs: [expected], pdf, outDir: out, label: values.label, subject: `${fixture.name} island ${index} (printed by ${values.printer})` });
  // What the chart says: the SVG's strings (as <text> or outline labels) against Chrome's.
  const textsFile = refs.texts.find((f) => path.basename(f) === `island-${index}.texts.json`);
  if (textsFile) {
    result.text = compareTexts(JSON.parse(fs.readFileSync(textsFile, "utf8")), svg);
    result.pass = result.pass && result.text.pass;
    fs.writeFileSync(path.join(out, "score.json"), `${JSON.stringify(result, null, 2)}\n`);
  } else {
    console.log(`note: no reference/island-${index}.texts.json; text not compared`);
  }
}
printResult(result);
process.exit(result.pass || values["report-only"] ? 0 : 1);
