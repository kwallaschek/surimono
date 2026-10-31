// Unit tests for the comparison checks, on synthetic images and SVG strings
// (no Chrome, no poppler):  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import test from "node:test";
import { PNG } from "pngjs";
import { verdictOf } from "../lib/compare.mjs";
import { colourMatch, diff, hueMatch, nearMatch, solidMask } from "../lib/png.mjs";
import { compareStyles, compareTexts, decodeXml, multisetMinus, svgTextStyles, svgTexts } from "../lib/svgtext.mjs";

const W = 96;
const H = 64;

/** A white image with the given rectangles [x, y, w, h, [r, g, b]] painted on it. */
function image(rects) {
  const png = new PNG({ width: W, height: H });
  png.data.fill(255);
  for (const [x0, y0, w, h, c] of rects) {
    for (let y = y0; y < y0 + h; y++) {
      for (let x = x0; x < x0 + w; x++) png.data.set([...c, 255], (y * W + x) * 4);
    }
  }
  return png;
}

const BLUE = [0x2f, 0x6f, 0xb0];
const ORANGE = [0xe0, 0x7b, 0x39];
const GOLD = [0xd4, 0xa7, 0x2c]; // the palette's closest pair: 44 from ORANGE
/** Anti-aliased tint of `c` at `coverage` over white. */
const tint = (c, coverage) => c.map((v) => Math.round(coverage * v + (1 - coverage) * 255));

test("solid pixels are flat interiors, never edges or thin lines", () => {
  const img = image([
    [10, 10, 20, 20, BLUE],
    [50, 10, 1, 40, BLUE],
  ]);
  const mask = solidMask(img);
  assert.equal(mask[20 * W + 20], 1, "bar interior");
  assert.equal(mask[20 * W + 10], 0, "bar edge");
  assert.equal(mask[20 * W + 50], 0, "1 px line");
});

test("near-match is colour-blind; the colour check is not", () => {
  const a = image([[10, 10, 40, 40, ORANGE]]);
  const b = image([[10, 10, 40, 40, GOLD]]);
  assert.equal(nearMatch(a, b).worst_window, 0, "orange and gold are within 128 per channel");
  assert.equal(colourMatch(a, b).colour_worst_window, 1);
  const { score } = diff(a, b);
  assert.deepEqual(verdictOf(score), { pass: false, failed: ["colour", "hue"] });
});

test("the same picture passes every check", () => {
  const a = image([[10, 10, 40, 40, BLUE]]);
  const { score } = diff(a, image([[10, 10, 40, 40, BLUE]]));
  assert.deepEqual(verdictOf(score), { pass: true, failed: [] });
});

test("a bar shifted by 1 px keeps its colour score at 0", () => {
  const { score } = diff(image([[10, 10, 40, 40, BLUE]]), image([[11, 10, 40, 40, BLUE]]));
  assert.equal(score.colour_worst_window, 0);
  assert.equal(score.hue_worst_window, 0);
});

test("hue ignores anti-aliasing coverage but sees a different colour", () => {
  // A 1 px line: full colour in one image, a 40% tint (anti-aliased) in the other.
  const full = image([[30, 5, 1, 50, BLUE]]);
  const aa = image([[30, 5, 1, 50, tint(BLUE, 0.4)]]);
  assert.equal(hueMatch(full, aa).hue_worst_window, 0);
  const orange = image([[30, 5, 1, 50, ORANGE]]);
  assert.equal(hueMatch(full, orange).hue_worst_window, 1);
});

test("hue does not judge position: a line with no ink near it in the other image is near-match's job", () => {
  const r = hueMatch(image([[20, 5, 1, 50, BLUE]]), image([[60, 5, 1, 50, BLUE]]));
  assert.equal(r.hue_mismatch_pixels, 0);
});

test("svgTexts reads <text> with tspans, entities, labels and titles", () => {
  const svg = `<svg><text x="1">Q1 &amp; Q2</text><text><tspan>売上</tspan><tspan>高</tspan></text>
    <g aria-label="X-axis titled 'Month'"><title>tip</title></g><text/></svg>`;
  assert.deepEqual(svgTexts(svg), { text: ["Q1&Q2", "売上高"], labels: ["X-axistitled'Month'", "tip"] });
  assert.equal(decodeXml("&#x6708;&#26376;&lt;"), "月月<");
});

test("svgTexts: a self-closing <text/> doesn't swallow the next element", () => {
  const svg = `<svg><text x="1"/><text>Jan</text><title/><g><title>tip</title></g></svg>`;
  assert.deepEqual(svgTexts(svg), { text: ["Jan"], labels: ["tip"] });
});

test("svgTexts: a <title> inside <text> is a label, not printed text", () => {
  // Same input as p1-gfx-05's outline test, whose aria-label is "Hi & bye".
  const svg = `<svg><text>Hi &amp; <tspan>bye</tspan><title>tip</title></text><text>a<desc>d</desc>b</text></svg>`;
  assert.deepEqual(svgTexts(svg), { text: ["Hi&bye", "ab"], labels: ["tip"] });
  const outlined = `<svg><g role="img" aria-label="Hi &amp; bye"><path d="M0 0"/></g><g aria-label="ab"><path d="M0 0"/></g></svg>`;
  assert.equal(compareTexts(svgTexts(svg), outlined).pass, true);
});

test("text check accepts outlines that keep the string in aria-label", () => {
  const chrome = `<svg><g aria-label="chart"><text>Jan</text><text>1,200</text></g></svg>`;
  const outlined = `<svg><g aria-label="chart"><g aria-label="Jan"><path d="M0 0"/></g><g><title>1,200</title><path d="M0 0"/></g></g></svg>`;
  const r = compareTexts(svgTexts(chrome), outlined);
  assert.equal(r.pass, true, JSON.stringify(r));
  assert.equal(r.candidate_outlined, 2);
});

test("text check fails on a changed, missing or extra string", () => {
  const ref = svgTexts(`<svg><text>Jan</text><text>1,200</text></svg>`);
  assert.deepEqual(compareTexts(ref, `<svg><text>Jan</text><text>1,201</text></svg>`).missing, ["1,200"]);
  assert.equal(compareTexts(ref, `<svg><text>Jan</text></svg>`).pass, false);
  assert.deepEqual(compareTexts(ref, `<svg><text>Jan</text><text>1,200</text><text>Feb</text></svg>`).extra, ["Feb"]);
});

test("multisetMinus counts duplicates", () => {
  assert.deepEqual(multisetMinus(["a", "a", "b"], ["a"]), ["a", "b"]);
});

test("svgTextStyles resolves weight and style from attributes, style, the font shorthand and ancestors", () => {
  const svg = `<svg font-weight="bold"><g style="font-style:italic"><text>A</text></g>
    <text font-weight="normal">B</text><text style="font: 600 12px 'Noto Sans'">C</text>
    <text style="font: 12px sans-serif">D</text><g font-weight="300"><text font-weight="bolder">E</text></g>
    <text aria-label="label F" style="font-weight:700;">F</text><text/></svg>`;
  assert.deepEqual(svgTextStyles(svg), [
    { key: "A", weight: 700, style: "italic" },
    { key: "B", weight: 400, style: "normal" },
    { key: "C", weight: 600, style: "normal" },
    { key: "D", weight: 400, style: "normal" },
    { key: "E", weight: 400, style: "normal" },
    { key: "labelF", weight: 700, style: "normal" },
  ]);
});

test("font check: outlines that keep the text's attributes pass; a bold label fails; missing keys are the text check's", () => {
  const chrome = `<svg><text style="font-size:12px;font-weight:bold">Title</text><text>Jan</text><text aria-label="Value: 3">3</text></svg>`;
  const ref = svgTextStyles(chrome);
  const outlined = `<svg><g style="font-size:12px;font-weight:bold" role="img" aria-label="Title"><path d="M0 0"/></g>
    <g role="img" aria-label="Jan"><path d="M0 0"/></g><g aria-label="Value: 3" role="graphics-symbol"><path d="M0 0"/></g></svg>`;
  assert.deepEqual(compareStyles(ref, outlined), { pass: true, compared: 3, mismatches: [], missing: [] });
  const bold = outlined.replace('aria-label="Jan"', 'aria-label="Jan" font-weight="bold"');
  const r = compareStyles(ref, bold);
  assert.equal(r.pass, false);
  assert.deepEqual(r.mismatches, [{ key: "Jan", chrome: "400 normal", surimono: "700 normal" }]);
  // The calibration fault: every <text> wrapped in <g font-weight="bold">.
  const fault = chrome.replace(/<text\b[\s\S]*?<\/text>/g, (t) => `<g font-weight="bold">${t}</g>`);
  assert.equal(compareStyles(ref, fault).pass, false);
  // A text the candidate lacks is missing, and fails.
  const lacking = compareStyles(ref, "<svg><text>Other</text></svg>");
  assert.equal(lacking.pass, false);
  assert.equal(lacking.missing.length, 3);
});

test("font check matches same-key texts one to one", () => {
  // echarts-pie-ja: each legend entry and its slice label share a string.
  const chrome = `<svg><text>East</text><text>East</text><text>West</text><text>West</text></svg>`;
  const ref = svgTextStyles(chrome);
  const outline = (key, bold) => `<g role="img" aria-label="${key}"${bold ? ' font-weight="bold"' : ""}><path d="M0 0"/></g>`;
  const good = `<svg>${outline("East")}${outline("East")}${outline("West")}${outline("West")}</svg>`;
  assert.equal(compareStyles(ref, good).pass, true);
  // Slice labels bold, legend entries normal: one of each pair is wrong.
  const r = compareStyles(ref, `<svg>${outline("East", true)}${outline("East")}${outline("West", true)}${outline("West")}</svg>`);
  assert.equal(r.pass, false);
  assert.deepEqual(r.mismatches, [
    { key: "East", chrome: "400 normal", surimono: "700 normal" },
    { key: "West", chrome: "400 normal", surimono: "700 normal" },
  ]);
  // Two candidates for one reference text: the right one is taken, the extra is ignored.
  assert.equal(compareStyles(svgTextStyles("<svg><text>East</text></svg>"), `<svg>${outline("East", true)}${outline("East")}</svg>`).pass, true);
});

test("font check candidates are <text> and outline groups, never chart marks", () => {
  const ref = svgTextStyles(`<svg><text>Jan</text></svg>`);
  // A labelled mark, or a labelled group of labelled marks, doesn't stand in for the text.
  for (const svg of [
    `<svg><path aria-label="Jan" role="graphics-symbol" d="M0 0"/></svg>`,
    `<svg><g aria-label="Jan"><path aria-label="bar" role="graphics-symbol" d="M0 0"/></g></svg>`,
    `<svg><g aria-label="Jan"><rect width="1" height="1"/></g></svg>`,
  ]) {
    const r = compareStyles(ref, svg);
    assert.equal(r.pass, false, svg);
    assert.equal(r.missing.length, 1, svg);
  }
  // gfx's outline of a <text> with tspans: nested unlabelled groups of paths.
  assert.equal(compareStyles(ref, `<svg><g role="img" aria-label="Jan"><g><path d="M0 0"/></g><path d="M1 1"/></g></svg>`).pass, true);
});

test("computedTextStyles is Chrome's view, and catches what svgTextStyles can't read", async (t) => {
  const { createRequire } = await import("node:module");
  const fs = await import("node:fs");
  const require = createRequire(import.meta.url);
  let chromium;
  try {
    ({ chromium } = require("playwright"));
    if (!fs.existsSync(chromium.executablePath())) return t.skip("Playwright's Chromium is not installed");
  } catch (e) {
    return t.skip(`playwright not available: ${e.message}`);
  }
  const { computedTextStyles } = await import("../lib/fixtures.mjs");
  const browser = await chromium.launch({ channel: "chromium" });
  try {
    const plain = `<svg xmlns="http://www.w3.org/2000/svg"><g font-weight="bold"><text>A</text></g><text style="font: italic 600 12px serif">B</text><text aria-label="C label">C</text></svg>`;
    const sort = (xs) => xs.map((x) => `${x.key}|${x.weight}|${x.style}`).sort();
    assert.deepEqual(sort(await computedTextStyles(browser, plain)), sort(svgTextStyles(plain)));
    // A <style> sheet, which the resolver doesn't read: reference.mjs would refuse it.
    const sheet = `<svg xmlns="http://www.w3.org/2000/svg"><style>text { font-weight: bold }</style><text>A</text></svg>`;
    assert.notDeepEqual(sort(await computedTextStyles(browser, sheet)), sort(svgTextStyles(sheet)));
  } finally {
    await browser.close();
  }
});
