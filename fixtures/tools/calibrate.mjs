#!/usr/bin/env node
// Calibrates the diff threshold: takes each fixture's Chrome island SVG,
// applies small known faults, prints the faulty SVG alone in Chrome and scores
// it against the island reference. A good threshold passes the harmless faults
// (sub-pixel text shifts) and fails the real ones (wrong font, missing labels,
// wrong bar lengths).
//
//   node calibrate.mjs --out DIR [fixture ...]     (needs DIR from reference.mjs)

import fs from "node:fs";
import path from "node:path";
import { parseArgs } from "node:util";
import { launchChromium, listFixtures, printIslandInChrome, rasterise } from "./lib/fixtures.mjs";
import { PASS_WORST_WINDOW, referenceFiles } from "./lib/compare.mjs";
import { diff, readPng } from "./lib/png.mjs";

const { values, positionals } = parseArgs({ allowPositionals: true, options: { out: { type: "string" } } });
const outRoot = path.resolve(values.out);

/** Each fault rewrites the SVG text. Expected verdict in brackets. */
const eachText = (s, f) => s.replace(/<text\b[\s\S]*?<\/text>/g, f);
const wrapText = (attrs) => (s) => eachText(s, (t) => `<g ${attrs}>${t}</g>`);
const FAULTS = {
  "text shifted 0.5px right [harmless]": wrapText('transform="translate(0.5 0)"'),
  "text shifted 0.5px down [harmless]": wrapText('transform="translate(0 0.5)"'),
  "text shifted 1px right [harmless]": wrapText('transform="translate(1 0)"'),
  "text 2% wider spacing [borderline]": wrapText('letter-spacing="0.2"'),
  "text shifted 3px right [fault]": wrapText('transform="translate(3 0)"'),
  "wrong font: Noto Sans JP for Latin [fault]": (s) => s.replace(/Noto Sans(?! JP)/g, "Noto Sans JP"),
  "wrong weight: all text bold [fault]": wrapText('font-weight="bold"'),
  "one label in four missing [fault]": (s) => {
    let i = 0;
    return eachText(s, (t) => (i++ % 4 === 3 ? "" : t));
  },
  "whole chart 1% smaller [fault]": (s) => s.replace(/(<svg\b[^>]*>)/, '$1<g transform="scale(0.99)">').replace(/<\/svg>\s*$/, "</g></svg>"),
};

const browser = await launchChromium();
const rows = [];
try {
  for (const fixture of listFixtures("islands", positionals)) {
    const refs = referenceFiles(fixture.dir);
    for (const [index, refPng] of refs.islands.entries()) {
      const svgFile = path.join(outRoot, fixture.name, `island-${index}.svg`);
      if (!fs.existsSync(svgFile)) continue;
      const svg = fs.readFileSync(svgFile, "utf8");
      const m = /<svg\b[^>]*?\swidth="([\d.]+)"[^>]*?\sheight="([\d.]+)"/.exec(svg);
      const expected = readPng(refPng);
      for (const [name, fault] of Object.entries(FAULTS)) {
        const work = path.join(outRoot, fixture.name, "calibrate");
        fs.mkdirSync(work, { recursive: true });
        const slug = name.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
        const pdf = path.join(work, `${index}-${slug}.pdf`);
        await printIslandInChrome(browser, fixture, index, fault(svg), Number(m[1]), Number(m[2]), pdf);
        const [png] = rasterise(pdf, work, `${index}-${slug}`);
        const actual = readPng(png);
        const { score } = diff(expected, actual);
        const verdict = score.worst_window <= PASS_WORST_WINDOW ? "pass" : "FAIL";
        rows.push({ fixture: fixture.name, island: index, fault: name, worst_window: score.worst_window, near_ink_share: score.near_ink_share, pixelmatch_diff_ratio_ink: score.diff_ratio_ink, verdict });
        console.log(`${verdict}  worst window ${(score.worst_window * 100).toFixed(1).padStart(5)}%  pixelmatch ${(score.diff_ratio_ink * 100).toFixed(2).padStart(5)}%  ${fixture.name}#${index}  ${name}`);
      }
    }
  }
} finally {
  await browser.close();
}
fs.writeFileSync(path.join(outRoot, "calibration.json"), `${JSON.stringify(rows, null, 2)}\n`);
