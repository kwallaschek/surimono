#!/usr/bin/env node
// Calibrates the diff threshold: takes each fixture's Chrome island SVG,
// applies small known faults, prints the faulty SVG alone in Chrome and scores
// it against the island reference. A good threshold passes the harmless faults
// (sub-pixel text shifts) and fails the real ones (wrong font, missing labels,
// wrong bar lengths, wrong colours, changed text). Every check is reported:
// near-match (shape), the solid-colour and hue checks (colour) and the text
// comparison against Chrome's own strings.
//
//   node calibrate.mjs --out DIR [fixture ...]     (needs DIR from reference.mjs)

import fs from "node:fs";
import path from "node:path";
import { parseArgs } from "node:util";
import { launchChromium, listFixtures, printIslandInChrome, rasterise } from "./lib/fixtures.mjs";
import { PASS_COLOUR_WINDOW, PASS_HUE_WINDOW, PASS_WORST_WINDOW, referenceFiles } from "./lib/compare.mjs";
import { diff, nearMatch, readPng } from "./lib/png.mjs";
import { compareTexts, svgTexts } from "./lib/svgtext.mjs";

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
  "colours 3% darker [harmless]": (s) => mapColours(s, (c) => c.map((v) => Math.round(v * 0.97))),
  // A fault that doesn't apply to a fixture returns null (reported as n/a).
  "two most used series colours swapped [fault]": (s) => {
    const [a, b] = seriesColours(s);
    return b ? mapColours(s, (c, key) => (key === a ? hex2rgb(b) : key === b ? hex2rgb(a) : c)) : null;
  },
  "most used series colour -> nearest palette colour [fault]": (s) => {
    const [a] = seriesColours(s);
    if (!a) return null;
    const near = nearestOther(a);
    return mapColours(s, (c, key) => (key === a ? hex2rgb(near) : c));
  },
  "one number label changed [fault]": (s) => {
    let done = false;
    return eachText(s, (t) => (done ? t : t.replace(/>([^<]*?)(\d)([^<]*)</, (m, pre, d, post) => ((done = true), `>${pre}${(Number(d) + 1) % 10}${post}<`))));
  },
};

// Colours: every #rgb, #rrggbb and rgb()/rgba() in the SVG, mapped by key (#rrggbb).
const PALETTE = ["#2f6fb0", "#e07b39", "#3a9b6a", "#c2455a", "#7a5cb8", "#8c8c8c", "#d4a72c", "#4bb3c4", "#9db7d5"];
const COLOUR_RE = /#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b|rgba?\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*(?:,\s*[\d.]+\s*)?\)/g;
const hex2rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const rgb2hex = (c) => `#${c.map((v) => Math.max(0, Math.min(255, v)).toString(16).padStart(2, "0")).join("")}`;
function parseColour(t) {
  if (t[0] === "#") return t.length === 4 ? [...t.slice(1)].map((x) => parseInt(x + x, 16)) : hex2rgb(t.toLowerCase());
  return t.match(/\d+/g).slice(0, 3).map(Number);
}
function mapColours(svg, f) {
  return svg.replace(COLOUR_RE, (t) => {
    const c = parseColour(t);
    const key = rgb2hex(c);
    const out = f(c, key);
    return out === c ? t : rgb2hex(out);
  });
}
/**
 * Chromatic colours (not white, black or grey) used at least twice, or used as
 * a gradient stop (a gradient fill is one series drawn from one <stop> each),
 * most used first.
 */
function seriesColours(svg) {
  const n = new Map();
  for (const t of svg.match(COLOUR_RE) ?? []) {
    const c = parseColour(t);
    if (Math.max(...c) - Math.min(...c) < 24) continue;
    const key = rgb2hex(c);
    n.set(key, (n.get(key) ?? 0) + 1);
  }
  const stops = new Set();
  for (const [, t] of svg.matchAll(/stop-color\s*[=:]\s*["']?\s*(#[0-9a-f]{3,6}\b|rgba?\([^)]*\))/gi)) stops.add(rgb2hex(parseColour(t)));
  return [...n.entries()]
    .filter(([key, count]) => count >= 2 || stops.has(key))
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
    .map(([k]) => k);
}
/** The palette colour closest to `key` (largest channel difference), other than itself. */
function nearestOther(key) {
  const c = hex2rgb(key);
  return PALETTE.filter((p) => p !== key)
    .map((p) => [Math.max(...hex2rgb(p).map((v, i) => Math.abs(v - c[i]))), p])
    .sort((a, b) => a[0] - b[0])[0][1];
}

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
        const faulty = fault(svg);
        if (faulty === null) {
          rows.push({ fixture: fixture.name, island: index, fault: name, verdict: "n/a" });
          console.log(`${"n/a".padEnd(18)} ${fixture.name}#${index}  ${name}`);
          continue;
        }
        const work = path.join(outRoot, fixture.name, "calibrate");
        fs.mkdirSync(work, { recursive: true });
        const slug = name.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
        const pdf = path.join(work, `${index}-${slug}.pdf`);
        await printIslandInChrome(browser, fixture, index, faulty, Number(m[1]), Number(m[2]), pdf);
        const [png] = rasterise(pdf, work, `${index}-${slug}`);
        const actual = readPng(png);
        const { score } = diff(expected, actual);
        const text = compareTexts(svgTexts(svg), faulty);
        const fails = [
          score.worst_window > PASS_WORST_WINDOW && "shape",
          score.colour_worst_window > PASS_COLOUR_WINDOW && "colour",
          score.hue_worst_window > PASS_HUE_WINDOW && "hue",
          !text.pass && "text",
        ].filter(Boolean);
        const verdict = fails.length ? `FAIL(${fails.join("+")})` : "pass";
        const alt = process.env.CALIBRATE_TOLERANCE ? nearMatch(expected, actual, { tolerance: Number(process.env.CALIBRATE_TOLERANCE) }).worst_window : undefined;
        rows.push({ fixture: fixture.name, island: index, fault: name, worst_window: score.worst_window, colour_worst_window: score.colour_worst_window, hue_worst_window: score.hue_worst_window, text_pass: text.pass, text_missing: text.missing, near_ink_share: score.near_ink_share, pixelmatch_diff_ratio_ink: score.diff_ratio_ink, verdict, alt_tolerance_worst_window: alt });
        const pct = (x) => `${(x * 100).toFixed(1).padStart(5)}%`;
        console.log(`${verdict.padEnd(18)} shape ${pct(score.worst_window)} colour ${pct(score.colour_worst_window)} hue ${pct(score.hue_worst_window)} text ${text.pass ? "ok  " : "DIFF"}${alt === undefined ? "" : ` (tolerance ${process.env.CALIBRATE_TOLERANCE}: ${pct(alt)})`}  ${fixture.name}#${index}  ${name}`);
      }
    }
  }
} finally {
  await browser.close();
}
fs.writeFileSync(path.join(outRoot, "calibration.json"), `${JSON.stringify(rows, null, 2)}\n`);

// Summary per fault: min and max of each score and how many fixtures fail.
const faults = [...new Set(rows.map((r) => r.fault))];
const range = (xs) => `${(Math.min(...xs) * 100).toFixed(1)} to ${(Math.max(...xs) * 100).toFixed(1)}%`;
const md = [
  "| Fault | Shape (near-match) | Colour (solid) | Hue (strokes) | Text differs | Fails |",
  "|---|---|---|---|---|---|",
  ...faults.map((f) => {
    const rs = rows.filter((r) => r.fault === f && r.verdict !== "n/a");
    return `| ${f} | ${range(rs.map((r) => r.worst_window))} | ${range(rs.map((r) => r.colour_worst_window))} | ${range(rs.map((r) => r.hue_worst_window))} | ${rs.filter((r) => !r.text_pass).length}/${rs.length} | ${rs.filter((r) => r.verdict !== "pass").length}/${rs.length} |`;
  }),
  "",
].join("\n");
fs.writeFileSync(path.join(outRoot, "calibration.md"), md);
console.log(`\n${md}`);
