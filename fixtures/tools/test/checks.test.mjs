// Unit tests for the comparison checks, on synthetic images and SVG strings
// (no Chrome, no poppler):  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import test from "node:test";
import { PNG } from "pngjs";
import { verdictOf } from "../lib/compare.mjs";
import { islandWrapper, islandsOf, prepassHtml } from "../lib/fixtures.mjs";
import { colourMatch, diff, hueMatch, nearMatch, solidMask } from "../lib/png.mjs";
import { compareStyles, compareTexts, decodeXml, expectedFace, faceName, fixtureFaces, matchWeight, multisetMinus, stampedOutlines, svgTextStyles, svgTexts } from "../lib/svgtext.mjs";

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

test("font check: outlines that keep the text's attributes pass; a bold label fails; a missing key fails", () => {
  const chrome = `<svg><text style="font-size:12px;font-weight:bold">Title</text><text>Jan</text><text aria-label="Value: 3">3</text></svg>`;
  const ref = svgTextStyles(chrome);
  const outlined = `<svg><g style="font-size:12px;font-weight:bold" role="img" aria-label="Title"><path d="M0 0"/></g>
    <g role="img" aria-label="Jan"><path d="M0 0"/></g><g aria-label="Value: 3" role="graphics-symbol"><path d="M0 0"/></g></svg>`;
  assert.deepEqual(compareStyles(ref, outlined), { pass: true, compared: 3, faces_checked: 0, mismatches: [], missing: [], extra: [] });
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
  // Two candidates for one reference text: the right one is taken, and the other is extra (P1-CF-11).
  const two = compareStyles(svgTextStyles("<svg><text>East</text></svg>"), `<svg>${outline("East", true)}${outline("East")}</svg>`);
  assert.equal(two.pass, false);
  assert.deepEqual(two.extra, [{ key: "East", surimono: "700 normal" }]);
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

test("text check: every copy counts, however it is drawn", () => {
  // Review of #43: a second copy of a label, or a string drawn both as an
  // outline and as a <text>, passed because copies cancelled out.
  const ref = svgTexts(`<svg><text>East</text><text>West</text></svg>`);
  const outline = (key, bold) => `<g role="img" aria-label="${key}"${bold ? ' font-weight="bold"' : ""}><path d="M0 0"/></g>`;
  assert.equal(compareTexts(ref, `<svg>${outline("East")}${outline("West")}</svg>`).pass, true);
  const twice = compareTexts(ref, `<svg>${outline("East")}${outline("West")}${outline("East", true)}</svg>`);
  assert.equal(twice.pass, false);
  assert.deepEqual(twice.extra, ["East"]);
  const both = compareTexts(ref, `<svg>${outline("East")}${outline("West")}<text>East</text></svg>`);
  assert.equal(both.pass, false);
  assert.deepEqual(both.extra, ["East"]);
  // A label Chrome draws twice (echarts-pie-ja: legend entry and slice label) needs two copies, no more.
  const pie = svgTexts(`<svg><text>East</text><text>East</text></svg>`);
  assert.equal(compareTexts(pie, `<svg>${outline("East")}${outline("East")}</svg>`).pass, true);
  assert.deepEqual(compareTexts(pie, `<svg>${outline("East")}</svg>`).missing, ["East"]);
  assert.deepEqual(compareTexts(pie, `<svg>${outline("East")}${outline("East")}${outline("East")}</svg>`).extra, ["East"]);
  // Labels Chrome's SVG has too are the chart library's own, not drawn strings.
  assert.equal(compareTexts(svgTexts(`<svg aria-label="chart"><text>East</text></svg>`), `<svg aria-label="chart">${outline("East")}</svg>`).pass, true);
});

test("font check: an extra copy of a label, or a key Chrome has no text for, fails", () => {
  const ref = svgTextStyles(`<svg><text>East</text><text>West</text></svg>`);
  const outline = (key, bold) => `<g role="img" aria-label="${key}"${bold ? ' font-weight="bold"' : ""}><path d="M0 0"/></g>`;
  const r = compareStyles(ref, `<svg>${outline("East")}${outline("West")}${outline("East", true)}</svg>`);
  assert.equal(r.pass, false);
  assert.deepEqual(r.mismatches, []);
  assert.deepEqual(r.missing, []);
  assert.deepEqual(r.extra, [{ key: "East", surimono: "700 normal" }]);
  const other = compareStyles(ref, `<svg>${outline("East")}${outline("West")}<text>North</text></svg>`);
  assert.equal(other.pass, false);
  assert.deepEqual(other.extra, [{ key: "North", surimono: "400 normal" }]);
});

test("outline groups: the group's own role doesn't matter, a labelled or role-carrying descendant does", () => {
  // gfx keeps the role of the <text> it outlines (Vega's text marks carry
  // role="graphics-symbol"), so a labelled group of plain paths counts whatever its role.
  const keys = (svg) => svgTextStyles(svg, { outlines: true }).map((x) => x.key);
  assert.deepEqual(keys(`<svg><g role="graphics-symbol" aria-label="Value: 3"><path d="M0 0"/></g></svg>`), ["Value:3"]);
  assert.deepEqual(keys(`<svg><g role="img" aria-label="Jan"><g><path d="M0 0"/></g></g></svg>`), ["Jan"]);
  assert.deepEqual(keys(`<svg><g aria-label="Jan"><g role="graphics-object"><path d="M0 0"/></g></g></svg>`), []);
  assert.deepEqual(keys(`<svg><g aria-label="Jan"><path role="graphics-symbol" d="M0 0"/></g></svg>`), []);
  assert.deepEqual(keys(`<svg><g aria-label="Jan"><path aria-label="bar" d="M0 0"/></g></svg>`), []);
  // Without `outlines`, only <text> counts.
  assert.deepEqual(svgTextStyles(`<svg><g role="img" aria-label="Jan"><path d="M0 0"/></g></svg>`), []);
});

// ---------------------------------------------------------------- gfx's stamps (P1-CF-12)
// The outline groups below are built the way p1-gfx-05's crates/gfx/src/svg/text.rs
// (convert_text) writes them: data-surimono-text (the outlined string) and
// data-surimono-face (the faces, ", "-joined) always; the author's aria-label
// and role kept; role="img" and aria-label = the string only when the <text>
// had neither.

/** gfx's outline group for a <text>: `label`/`role` are the author's, if any. */
const stampedOutline = (text, faces, { label, role, attrs = "", inner = '<path d="M0 0"/>' } = {}) => {
  const own = label === undefined ? `${role === undefined ? ' role="img"' : ` role="${role}"`} aria-label="${text}"` : `${role === undefined ? "" : ` role="${role}"`} aria-label="${label}"`;
  return `<g${attrs} data-surimono-text="${text}" data-surimono-face="${faces}"${own}>${inner}</g>`;
};

test("text check reads the drawn string from data-surimono-text, so an author's aria-label can't hide it", () => {
  // vegalite-bar-simple-latin: each value label is <text aria-label="…" role="graphics-symbol">.
  const chrome = `<svg><text aria-label="value: 14,695" role="graphics-symbol">14,695</text><text>Jan</text></svg>`;
  const ref = svgTexts(chrome);
  const stamped = `<svg>${stampedOutline("14,695", "Noto Sans Regular", { label: "value: 14,695", role: "graphics-symbol" })}${stampedOutline("Jan", "Noto Sans Regular")}</svg>`;
  const r = compareTexts(ref, stamped);
  assert.equal(r.pass, true, JSON.stringify(r));
  assert.equal(r.candidate_stamped, 2);
  // Without the stamps (gfx before #27) the old rule applies, and the value label reads as missing.
  const unstamped = stamped.replace(/ data-surimono-(text|face)="[^"]*"/g, "");
  assert.deepEqual(compareTexts(ref, unstamped).missing, ["14,695"]);
  // The stamp is compared, not trusted: a different drawn string fails both ways.
  const wrong = compareTexts(ref, stamped.replace('data-surimono-text="14,695"', 'data-surimono-text="14695"'));
  assert.deepEqual([wrong.missing, wrong.extra], [["14,695"], ["14695"]]);
  // A stamped string drawn twice is extra; so is one drawn as a stamp and as a <text>.
  assert.deepEqual(compareTexts(ref, stamped.replace("</svg>", `${stampedOutline("Jan", "Noto Sans Regular")}</svg>`)).extra, ["Jan"]);
  assert.deepEqual(compareTexts(ref, stamped.replace("</svg>", "<text>Jan</text></svg>")).extra, ["Jan"]);
  // gfx's own aria-label (= the string) isn't counted a second time.
  assert.deepEqual(stampedOutlines(stamped), [{ text: "14,695", label: "value:14,695" }, { text: "Jan", label: "Jan" }]);
});

test("stamped strings are decoded and normalised like Chrome's", () => {
  const ref = svgTexts(`<svg><text>Hi &amp; <tspan>bye</tspan><title>tip</title></text><text></text></svg>`);
  const cand = `<svg>${stampedOutline("Hi &amp; bye", "Noto Sans Regular, Noto Sans Bold", { inner: '<path d="M0 0"/><g><path d="M1 1"/></g><title>tip</title>' })}<g data-surimono-text="" data-surimono-face="" role="img" aria-label=""/></svg>`;
  assert.equal(compareTexts(ref, cand).pass, true, JSON.stringify(compareTexts(ref, cand)));
});

test("font check: a stamped group is an outline whatever it holds", () => {
  // gfx copies a text's <title>, <desc> and <a> into its group (#50 review, nit 5).
  const ref = svgTextStyles(`<svg><text>Jan<title>January</title></text></svg>`);
  const withTitle = `<svg>${stampedOutline("Jan", "Noto Sans Regular", { inner: '<path d="M0 0"/><title>January</title>' })}</svg>`;
  const r = compareStyles(ref, withTitle);
  assert.equal(r.pass, true, JSON.stringify(r));
  assert.equal(r.faces_checked, 1);
  // Unstamped, the same group isn't an outline (the old rule), so the text is missing.
  assert.equal(compareStyles(ref, withTitle.replace(/ data-surimono-(text|face)="[^"]*"/g, "")).missing.length, 1);
});

test("matchWeight follows CSS Fonts 4 font matching", () => {
  const two = [400, 700];
  assert.deepEqual([100, 300, 400, 450, 500, 501, 600, 700, 900].map((w) => matchWeight(w, two)), [400, 400, 400, 400, 400, 700, 700, 700, 700]);
  const three = [300, 500, 700];
  assert.deepEqual([400, 450, 350, 550, 200, 900].map((w) => matchWeight(w, three)), [500, 500, 300, 700, 300, 700]);
  assert.deepEqual([400, 600].map((w) => matchWeight(w, [100, 900])), [100, 900]);
});

test("faces: names as the name table spells them; the fixture fonts' faces", () => {
  assert.equal(faceName({ family: "Noto Sans JP", weight: 700, style: "normal" }), "Noto Sans JP Bold");
  assert.equal(faceName({ family: "Noto Sans", weight: 400, style: "italic" }), "Noto Sans Italic");
  assert.equal(faceName({ family: "Noto Sans", weight: 700, style: "italic" }), "Noto Sans Bold Italic");
  assert.deepEqual(fixtureFaces().map(faceName).sort(), ["Noto Sans Bold", "Noto Sans JP Bold", "Noto Sans JP Regular", "Noto Sans Regular"]);
  const fonts = fixtureFaces();
  assert.equal(expectedFace("Noto Sans", 600, "normal", fonts), "Noto Sans Bold");
  assert.equal(expectedFace("Noto Sans JP", 400, "italic", fonts), "Noto Sans JP Regular (synthetic oblique)");
  assert.equal(expectedFace("Arial", 400, "normal", fonts), undefined);
  const withItalic = [...fonts, { family: "Noto Sans", weight: 400, style: "italic" }];
  assert.equal(expectedFace("Noto Sans", 400, "italic", withItalic), "Noto Sans Italic");
});

test("font check compares the faces gfx drew with what font matching picks for Chrome's weight and style", () => {
  // Chrome: a bold title, a normal label, an italic note, Japanese in a Noto Sans text.
  const chrome = `<svg font-family="Noto Sans"><text font-weight="bold">Title</text><text>Jan</text><text font-style="italic">note</text><text font-weight="600">売上 Sales</text></svg>`;
  const ref = svgTextStyles(chrome);
  const outline = (text, faces, attrs) => stampedOutline(text, faces, { attrs });
  const good = `<svg font-family="Noto Sans">${outline("Title", "Noto Sans Bold", ' font-weight="bold"')}${outline("Jan", "Noto Sans Regular")}${outline("note", "Noto Sans Regular (synthetic oblique)", ' font-style="italic"')}${outline("売上 Sales", "Noto Sans JP Bold, Noto Sans Bold", ' font-weight="600"')}</svg>`;
  const r = compareStyles(ref, good);
  assert.equal(r.pass, true, JSON.stringify(r));
  assert.equal(r.faces_checked, 4);
  const faultWith = (from, to) => compareStyles(ref, good.replace(from, to));
  // A gfx that ignores font-weight: the declared weight is right, the drawn face isn't.
  const ignored = faultWith('data-surimono-face="Noto Sans Bold"', 'data-surimono-face="Noto Sans Regular"');
  assert.equal(ignored.pass, false);
  assert.deepEqual(ignored.mismatches, [{ key: "Title", chrome: "700 normal", surimono: "drawn with Noto Sans Regular", expected: "Noto Sans Bold" }]);
  // A per-glyph fallback that picks the Bold face for a normal-weight label.
  assert.equal(faultWith('data-surimono-face="Noto Sans Regular"', 'data-surimono-face="Noto Sans JP Bold, Noto Sans Regular"').mismatches[0].expected, "Noto Sans JP Regular");
  // Italic not slanted; a face outside the fixture fonts; an unreadable face.
  assert.equal(faultWith("Noto Sans Regular (synthetic oblique)", "Noto Sans Regular").pass, false);
  assert.equal(faultWith('data-surimono-face="Noto Sans Regular"', 'data-surimono-face="DejaVu Sans Book"').mismatches[0].expected, "a fixture face");
  assert.equal(faultWith('data-surimono-face="Noto Sans Regular"', 'data-surimono-face="(unreadable face)"').pass, false);
  // A non-empty string drawn with no face is malformed output, and fails (#55 review).
  const noFace = compareStyles(svgTextStyles("<svg><text>Jan</text></svg>"), `<svg>${outline("Jan", "")}</svg>`);
  assert.equal(noFace.pass, false);
  assert.deepEqual(noFace.mismatches, [{ key: "Jan", chrome: "400 normal", surimono: "drawn with no face", expected: "a fixture face" }]);
  const absent = compareStyles(svgTextStyles("<svg><text>Jan</text></svg>"), '<svg><g data-surimono-text="Jan" role="img" aria-label="Jan"><path d="M0 0"/></g></svg>');
  assert.equal(absent.pass, false);
  // An empty string (an author label on an empty <text>) has no face to check.
  const empty = compareStyles(svgTextStyles('<svg><text aria-label="nothing"></text></svg>'), '<svg><g data-surimono-text="" data-surimono-face="" aria-label="nothing"/></svg>');
  assert.equal(empty.pass, true, JSON.stringify(empty));
  assert.equal(empty.faces_checked, 0);
});

test("text check: a stamped group's author label is still compared with Chrome's", () => {
  // #55 review: setting every stamped label aside let a wrong author label pass.
  const ref = svgTexts(`<svg><text aria-label="Revenue: 12" role="graphics-symbol">12</text></svg>`);
  const group = (label, role) => `<svg><g${role ? ` role="${role}"` : ""} aria-label="${label}" data-surimono-text="12" data-surimono-face="Noto Sans Regular"><path d="M0 0"/></g></svg>`;
  assert.equal(compareTexts(ref, group("Revenue: 12", "graphics-symbol")).pass, true);
  const wrongAuthor = compareTexts(ref, group("Revenue: 13", "graphics-symbol"));
  assert.equal(wrongAuthor.pass, false);
  assert.deepEqual(wrongAuthor.extra, ["Revenue:13"]);
  // gfx's own label disagreeing with its stamp: the label counts as drawn too.
  const wrongOwn = compareTexts(ref, group("13", "img"));
  assert.equal(wrongOwn.pass, false);
  assert.deepEqual(wrongOwn.extra, ["13"]);
});

test("stamps are read as attributes, never from inside another attribute's value", () => {
  // #55 review: data-surimono-text='X' written inside an aria-label read as a stamp.
  const forged = `<svg><g aria-label="note data-surimono-text='X' data-surimono-face='Noto Sans Regular'"><path d="M0 0"/></g></svg>`;
  assert.deepEqual(stampedOutlines(forged), []);
  const r = compareTexts(svgTexts("<svg><text>X</text></svg>"), forged);
  assert.equal(r.pass, false);
  assert.equal(r.candidate_stamped, 0);
  assert.deepEqual(svgTextStyles(forged, { outlines: true }).map((x) => x.faces), [undefined]);
  // Single-quoted stamps are attributes all the same.
  assert.deepEqual(stampedOutlines(`<svg><g data-surimono-text='X' aria-label='X'/></svg>`), [{ text: "X", label: "X" }]);
});

