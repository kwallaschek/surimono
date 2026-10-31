// Produces crates/gfx/tests/data/chrome_svg_text.json: where Chrome places
// every character of SVG <text>, with the shared test fonts. This is the
// outside truth for the text-to-outline layout in crates/gfx/src/svg/text.rs;
// never edit the JSON by hand, rerun this script.
//
// Usage (from the repo root):
//   node crates/gfx/tests/chrome/svg_text.mjs [fonts-dir] [out.json]
// Needs Playwright with its Chromium (NODE_PATH or a local node_modules).
//
// Fonts load with @font-face from the exact files in fixtures/fonts. For each
// <text> in a case (document order) and each addressable character i, the
// JSON holds getStartPositionOfChar(i), getEndPositionOfChar(i) (both on the
// unshifted baseline, in the text's user space) and getExtentOfChar(i) (the
// character's font box, which includes the dominant-baseline shift).

import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let playwright;
try { playwright = require('playwright'); } catch { playwright = require('/opt/npm-tools/node_modules/playwright'); }

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../../..');
const fontsDir = resolve(process.argv[2] ?? join(repo, 'fixtures/fonts'));
const outFile = resolve(process.argv[3] ?? join(repo, 'crates/gfx/tests/data/chrome_svg_text.json'));

const FONTS = [
  { file: 'NotoSans-Regular.ttf', family: 'Noto Sans', weight: 400 },
  { file: 'NotoSans-Bold.ttf', family: 'Noto Sans', weight: 700 },
  { file: 'NotoSansJP-Regular.otf', family: 'Noto Sans JP', weight: 400 },
  { file: 'NotoSansJP-Bold.otf', family: 'Noto Sans JP', weight: 700 },
];

const NS = "font-family=\"'Noto Sans'\"";
const JP = "font-family=\"'Noto Sans JP'\"";
const BASELINES = ['auto', 'alphabetic', 'central', 'middle', 'hanging', 'text-before-edge',
  'text-after-edge', 'ideographic', 'mathematical', 'text-top', 'text-bottom'];

// [name, svg content]. Every <text> names a registered family (Chrome would
// otherwise use system fonts) and the page sets no font of its own.
export const CASES = [
  ['latin', `<text x="10" y="30" ${NS} font-size="12">Hello, world</text>`],
  ['sizes', `<text x="10" y="30" ${NS} font-size="9pt">Points</text><g font-size="10"><text x="10" y="60" ${NS} font-size="1.5em">Em size</text><text x="10" y="90" ${NS} font-size="120%">Percent</text></g><text x="10" y="120" ${NS} font-size="13.5px">AVATAR Wave</text>`],
  ['default-size', `<text x="10" y="30" ${NS}>Sixteen</text>`],
  ['anchors', `<text x="200" y="30" ${NS} font-size="14" text-anchor="start">Start</text><text x="200" y="60" ${NS} font-size="14" text-anchor="middle">Middle</text><text x="200" y="90" ${NS} font-size="14" text-anchor="end">End</text>`],
  ['baselines', BASELINES.map((b, i) => `<text x="10" y="${30 + i * 25}" ${NS} font-size="20" dominant-baseline="${b}">Hxg ${b}</text>`).join('')],
  ['baselines-jp', ['auto', 'central', 'middle', 'hanging'].map((b, i) => `<text x="10" y="${30 + i * 30}" ${JP} font-size="16" dominant-baseline="${b}">売上高 ${b}</text>`).join('')],
  ['alignment-baseline', `<text x="10" y="40" ${NS} font-size="20">a<tspan alignment-baseline="central">b</tspan><tspan alignment-baseline="hanging">c</tspan><tspan dominant-baseline="central">d</tspan></text><text x="10" y="80" ${NS} font-size="20" alignment-baseline="central">e</text>`],
  ['japanese', `<text x="10" y="30" ${JP} font-size="12">売上高（百万円）</text><text x="10" y="60" ${JP} font-size="16" font-weight="bold">東京都品川区</text>`],
  ['fallback', `<text x="10" y="30" font-family="'Noto Sans', 'Noto Sans JP'" font-size="12">Sales 売上 2026年</text>`],
  ['weights-styles', `<text x="10" y="30" ${NS} font-size="14" font-weight="bold">Bold</text><text x="10" y="60" ${NS} font-size="14" font-weight="600">Six hundred</text><text x="10" y="90" ${NS} font-size="14" font-style="italic">Synthetic italic</text><g font-weight="bold"><text x="10" y="120" ${NS} font-size="14" font-weight="lighter">Lighter</text></g>`],
  ['letter-spacing', `<text x="10" y="30" ${NS} font-size="12" letter-spacing="2">Spaced out</text><text x="300" y="60" ${NS} font-size="12" letter-spacing="1.5px" text-anchor="middle">Centred spaced</text><text x="300" y="90" ${NS} font-size="12" style="letter-spacing:3px" text-anchor="end">End</text>`],
  ['dx-dy', `<text x="10" y="30" ${NS} font-size="14" dx="0 5 5" dy="0 2 -2">abcd</text><text x="10" y="60" ${NS} font-size="14" dx="3">shift</text>`],
  ['x-list', `<text x="10 40 70" y="30" ${NS} font-size="14">abcd</text><text x="300 320" y="60 70" ${NS} font-size="14" text-anchor="middle">xyz</text>`],
  ['tspans', `<text x="10" y="30" ${NS} font-size="14">Plain <tspan font-weight="bold">bold</tspan> tail</text><text x="200" y="60" ${NS} font-size="12" text-anchor="middle"><tspan x="200" dy="0">First line</tspan><tspan x="200" dy="1.2em">Second line</tspan></text><text x="10" y="120" ${NS} font-size="12">a<tspan dx="10" fill="red">b</tspan>c<tspan font-size="20" dy="-5">D</tspan>e</text>`],
  ['whitespace', `<text x="10" y="30" ${NS} font-size="14">  lead   and
    trail  </text><text x="10" y="60" ${NS} font-size="14">a <tspan> b</tspan> c</text><text x="10" y="90" ${NS} font-size="14" xml:space="preserve">  kept   spaces </text>`],
  ['inherited', `<g font-family="'Noto Sans'" font-size="20" text-anchor="middle" dominant-baseline="central" letter-spacing="1"><text x="200" y="40">Inherited</text><g style="font-size:12px;font-weight:bold"><text x="200" y="80">Nested style</text></g></g>`],
  ['style-attr', `<text x="10" y="30" style="font: italic bold 15px 'Noto Sans'">Shorthand</text><text x="10" y="60" font-size="30" style="font-size:12px;font-family:'Noto Sans';">Style wins</text>`],
  ['echarts', `<text dominant-baseline="central" text-anchor="end" style="font-size:12px;font-family:'Noto Sans';" transform="translate(32 230)" fill="#6E7079">40</text><text dominant-baseline="central" text-anchor="middle" style="font-size:12px;font-family:'Noto Sans', 'Noto Sans JP';" transform="translate(93 245)" fill="#6E7079">Mon 水</text><text dominant-baseline="central" text-anchor="start" style="font-size:18px;font-family:'Noto Sans', 'Noto Sans JP';font-weight:bold;" transform="translate(5 5)">売上 Sales</text>`],
  ['vega', `<text text-anchor="end" transform="translate(9.5,7) rotate(270) translate(0,3)" font-family="'Noto Sans', 'Noto Sans JP'" font-size="10px" fill="#000" opacity="1">日本</text><text text-anchor="middle" transform="translate(30,36)" font-family="'Noto Sans'" font-size="11px" font-weight="bold" fill="#000">a</text>`],
  ['ligatures-kerning', `<text x="10" y="30" ${NS} font-size="20">office affine AVATAR</text>`],
];

const sha = (b) => require('node:crypto').createHash('sha256').update(b).digest('hex');

const browser = await playwright.chromium.launch();
const version = browser.version();
const page = await browser.newPage();
await page.route('**/*', (route) => {
  const url = new URL(route.request().url());
  if (url.pathname.startsWith('/fonts/')) {
    return route.fulfill({ body: readFileSync(join(fontsDir, url.pathname.slice(7))), contentType: 'font/otf' });
  }
  const faces = FONTS.map((f) => `@font-face{font-family:'${f.family}';src:url(/fonts/${f.file});font-weight:${f.weight};font-style:normal}`).join('\n');
  return route.fulfill({ contentType: 'text/html', body: `<!doctype html><meta charset="utf-8"><style>${faces}\nbody{margin:0}</style><body><svg id="s" xmlns="http://www.w3.org/2000/svg" width="600" height="300"></svg>` });
});
await page.goto('https://svg-text.test/');
await page.evaluate(async (fonts) => {
  for (const f of fonts) await document.fonts.load(`${f.weight} 16px '${f.family}'`, 'a売');
  await document.fonts.ready;
}, FONTS);

const cases = [];
for (const [name, svg] of CASES) {
  const texts = await page.evaluate((markup) => {
    const s = document.getElementById('s');
    s.innerHTML = markup;
    const r = (v) => Math.round(v * 1e4) / 1e4;
    return [...s.querySelectorAll('text')].map((t) => {
      const chars = [];
      for (let i = 0; i < t.getNumberOfChars(); i++) {
        const a = t.getStartPositionOfChar(i);
        const b = t.getEndPositionOfChar(i);
        const e = t.getExtentOfChar(i);
        chars.push({ start: [r(a.x), r(a.y)], end: [r(b.x), r(b.y)], extent: [r(e.x), r(e.y), r(e.width), r(e.height)] });
      }
      return { text: t.textContent, length: r(t.getComputedTextLength()), chars };
    });
  }, svg);
  cases.push({ name, svg, texts });
}
await browser.close();

mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, JSON.stringify({
  generator: 'crates/gfx/tests/chrome/svg_text.mjs',
  chromium: version,
  playwright: require('playwright/package.json').version,
  fonts: FONTS.map((f) => { const b = readFileSync(join(fontsDir, f.file)); return { ...f, bytes: b.length, sha256: sha(b) }; }),
  cases,
}, null, 1) + '\n');
console.log(`wrote ${cases.length} cases to ${outFile} (Chromium ${version})`);
