// Unit tests for the comparison checks, on synthetic images and SVG strings
// (no Chrome, no poppler):  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import test from "node:test";
import { PNG } from "pngjs";
import { verdictOf } from "../lib/compare.mjs";
import { islandWrapper, islandsOf, prepassHtml } from "../lib/fixtures.mjs";
import { colourMatch, diff, hueMatch, nearMatch, solidMask } from "../lib/png.mjs";
import { compareTexts, decodeXml, multisetMinus, svgTexts } from "../lib/svgtext.mjs";

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

test("islandsOf cuts surimono's islands back out, in order, and reports error placeholders", () => {
  const marker = (r) => `<script type="application/vnd.surimono+json" data-renderer="${r}" class="chart wide">{}</script>`;
  const page = `<p>a</p>${marker("echarts")}<p>b</p>${marker("vega")}`;
  const svgs = ['<svg width="8" height="8"><g><text>A</text></g></svg>', '<svg width="8" height="8"><path d="M0 0"/></svg>'];
  assert.deepEqual(islandsOf(prepassHtml(page, svgs)), svgs.map((svg) => ({ svg })));
  const failed = `<div class="surimono-island surimono-island-error chart" data-error="time-limit"><!-- surimono: island 1 hit the time limit (2000 ms) --></div>`;
  const out = islandsOf(`${islandWrapper(marker("echarts"), svgs[0])}${failed}`);
  assert.deepEqual(out[0], { svg: svgs[0] });
  assert.match(out[1].error, /island 1 hit the time limit/);
});
