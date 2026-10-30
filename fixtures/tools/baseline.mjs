#!/usr/bin/env node
// The fulgur baseline: how close fulgur gets to Chrome when the islands are
// perfect, i.e. when it prints Chrome's own island SVGs.
//
//   node baseline.mjs --out DIR [fixture ...]
//
// Needs DIR from `node reference.mjs --out DIR` (prepass.html and
// island-<i>.svg per fixture). For each fixture, fulgur prints
//   - prepass.html (the page with Chrome's SVGs inlined)  -> compared with reference/chrome-<page>.png
//   - each island SVG alone at its size                   -> compared with reference/island-<i>.png
// Any difference comes from fulgur, not from surimono, so these scores are the
// floor that surimono + fulgur can reach. Writes DIR/baseline.json and
// DIR/baseline.md.

import fs from "node:fs";
import path from "node:path";
import { parseArgs } from "node:util";
import { islandPageHtml, listFixtures } from "./lib/fixtures.mjs";
import { comparePdf, printResult, referenceFiles, PASS_WORST_WINDOW } from "./lib/compare.mjs";
import { fulgurPrint, fulgurVersion } from "./lib/fulgur.mjs";

const { values, positionals } = parseArgs({ allowPositionals: true, options: { out: { type: "string" }, kind: { type: "string", default: "islands" } } });
if (!values.out) {
  console.error("usage: node baseline.mjs --out DIR [fixture ...]");
  process.exit(2);
}
const outRoot = path.resolve(values.out);
const version = fulgurVersion();
console.log(`fulgur ${version}`);

const rows = [];
for (const fixture of listFixtures(values.kind, positionals)) {
  const dir = path.join(outRoot, fixture.name);
  if (!fs.existsSync(path.join(dir, "prepass.html"))) {
    console.log(`skip ${fixture.name}: no prepass.html in ${dir} (run reference.mjs --out first)`);
    continue;
  }
  const refs = referenceFiles(fixture.dir);
  const work = path.join(dir, "fulgur");
  fs.mkdirSync(work, { recursive: true });
  const row = { fixture: fixture.name, page: null, islands: [] };
  try {
    fulgurPrint(path.join(dir, "prepass.html"), path.join(work, "page.pdf"), work);
    row.page = comparePdf({ expectedPngs: refs.pages, pdf: path.join(work, "page.pdf"), outDir: path.join(work, "page"), label: "fulgur", subject: `${fixture.name} page` });
    printResult(row.page);
  } catch (e) {
    row.page = { error: e.message };
    console.log(`ERROR ${fixture.name} page: ${e.message}`);
  }
  for (const [index, expected] of refs.islands.entries()) {
    const svgFile = path.join(dir, `island-${index}.svg`);
    const svg = fs.readFileSync(svgFile, "utf8");
    const m = /<svg\b[^>]*?\swidth="([\d.]+)"[^>]*?\sheight="([\d.]+)"/.exec(svg);
    const html = path.join(work, `island-${index}.html`);
    fs.writeFileSync(html, islandPageHtml(svg, Number(m[1]), Number(m[2])));
    try {
      fulgurPrint(html, path.join(work, `island-${index}.pdf`), work);
      const r = comparePdf({ expectedPngs: [expected], pdf: path.join(work, `island-${index}.pdf`), outDir: path.join(work, `island-${index}`), label: "fulgur", subject: `${fixture.name} island ${index}` });
      printResult(r);
      row.islands.push(r);
    } catch (e) {
      row.islands.push({ error: e.message });
      console.log(`ERROR ${fixture.name} island ${index}: ${e.message}`);
    }
  }
  rows.push(row);
}

const pct = (r) => (r?.worst_window === undefined ? "error" : `${(r.worst_window * 100).toFixed(1)}% (pixelmatch ${(r.worst_diff_ratio_ink * 100).toFixed(2)}%)`);
// The verdict is the tool's own: worst window at or below the threshold AND the
// same page size and count as Chrome (lib/compare.mjs). A size mismatch is named.
const verdict = (r) => {
  if (!r || r.error) return "FAIL (error)";
  if (r.pass) return "pass";
  const sizes = (r.pages ?? []).filter((p) => p.size_match === false).map((p) => `size ${p.actual_size} vs Chrome ${p.expected_size}`);
  const missing = (r.pages ?? []).filter((p) => p.worst_window === undefined).map((p) => p.detail);
  return `FAIL (${[...sizes, ...missing].join("; ") || "score"})`;
};
const islandsCell = (r) => r.islands.map(pct).join(", ");
const islandsVerdict = (r) => r.islands.map(verdict).join(", ");
const all = rows.flatMap((r) => [r.page, ...r.islands]);
const passed = all.filter((r) => r && !r.error && r.pass).length;
const md = [
  `# fulgur baseline (fulgur ${version})`,
  "",
  `fulgur printing Chrome's own island SVGs, compared with the Chrome references. Score: worst 32x32 window's share of near-mismatched ink (pass at or below ${(PASS_WORST_WINDOW * 100).toFixed(0)}% and same page size and count as Chrome), with pixelmatch's share of differing ink in brackets.`,
  "",
  "| Fixture | Page | Page verdict | Islands | Island verdict |",
  "|---|---|---|---|---|",
  ...rows.map((r) => `| ${r.fixture} | ${pct(r.page)} | ${verdict(r.page)} | ${islandsCell(r)} | ${islandsVerdict(r)} |`),
  "",
  `${passed} of ${all.length} comparisons pass.`,
  "",
].join("\n");
fs.writeFileSync(path.join(outRoot, "baseline.json"), `${JSON.stringify({ fulgur: version, rows }, null, 2)}\n`);
fs.writeFileSync(path.join(outRoot, "baseline.md"), md);
console.log(`\n${md}`);
