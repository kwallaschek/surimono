// Produces crates/gfx/tests/data/chrome_font_shorthand.json: whether Chrome's
// canvas accepts each CSS `font` shorthand, and the size in px it resolves to.
// These are the outside truth for the parser in crates/gfx/src/css_font.rs
// (test `chrome_validity_and_sizes`); never edit the JSON by hand, rerun this.
//
// Usage (from the repo root):
//   node crates/gfx/tests/chrome/font_shorthand.mjs [out.json]
// Needs Playwright with its Chromium (NODE_PATH or a local node_modules).
//
// Method: on a detached canvas, set a sentinel font, then the input. If
// ctx.font still reads as the sentinel, Chrome rejected the input ("invalid").
// Otherwise the size is read from the serialized font (Chrome serializes the
// computed size in px). A detached canvas resolves em/%/rem against 10px/16px.

import { createRequire } from 'node:module';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require('playwright');
} catch {
  playwright = require('/opt/npm-tools/node_modules/playwright');
}

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../../..');
const outFile = resolve(process.argv[2] ?? join(repo, 'crates/gfx/tests/data/chrome_font_shorthand.json'));

const INPUTS = [
  '1rem x', 'larger x', 'smaller x', 'medium x', 'xx-small x', 'xxx-large x',
  '0 x', '0px x', 'oblique 10deg 12px x', 'oblique 12px x', 'bolder 12px x',
  'lighter 12px x', 'small-caps 12px x', 'normal normal normal normal 12px x',
  'normal normal normal normal normal 12px x', 'bold bold 12px x', '600 12px x',
  '1000 12px x', '1001 12px x', '0.5 12px x', 'condensed 12px x',
  'ultra-expanded 12px x', '12px', '12px inherit', 'inherit', '12px/normal x',
  '12px / 1.5 x', '12px/-1 x', '-12px x', '12px 3D', '12px foo bar',
  '12px "a b", c', '12px "unterminated', '12px a,,b', '12px a,', '12px default',
  '12PX SANS-SERIF', 'BOLD 12px x', 'bold italic 12px x', '12.5e1px x',
  '1e1px x', '.5em x', '+12px x', '12px\tx', '12px\nx', 'caption',
  '12px \\66oo', '12px math', '12px initial', '100% x', '12px/1.2px x',
  '12px/50% x', 'semi-condensed bold italic small-caps 12px x',
  'italic italic 12px x', '1e6px x', '12px x !important', '12 px x',
  '12pxx x', "9pt 'Noto Sans'", "1.4em 'Noto Sans'", "150% 'Noto Sans'",
];

const browser = await playwright.chromium.launch();
const version = browser.version();
const page = await browser.newPage();
const cases = await page.evaluate((inputs) => {
  const ctx = document.createElement('canvas').getContext('2d');
  const SENTINEL = '13px sentinel-family-zzz';
  return inputs.map((input) => {
    ctx.font = SENTINEL;
    const before = ctx.font;
    ctx.font = input;
    const after = ctx.font;
    if (after === before && input !== SENTINEL) {
      return { input, valid: false, serialized: null, size_px: null };
    }
    const m = after.match(/(-?[0-9.]+(?:e[+-]?[0-9]+)?)px/);
    return { input, valid: true, serialized: after, size_px: m ? Number(m[1]) : null };
  });
}, INPUTS);
await browser.close();

mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, JSON.stringify({
  generator: 'crates/gfx/tests/chrome/font_shorthand.mjs',
  chromium: version,
  playwright: require('playwright/package.json').version,
  cases,
}, null, 2) + '\n');
console.log(`wrote ${cases.length} cases to ${outFile} (Chromium ${version})`);
