// Produces crates/gfx/tests/data/chrome_measure_text.json: Chrome's canvas
// measureText for fixed strings in the shared test fonts. These numbers are the
// outside truth for surimono-gfx's TextMeasurer; never edit the JSON by hand,
// rerun this script instead.
//
// Usage (from the repo root):
//   node crates/gfx/tests/chrome/measure_text.mjs [fonts-dir] [out.json]
// Defaults: fixtures/fonts and crates/gfx/tests/data/chrome_measure_text.json.
// Needs Playwright with its Chromium (NODE_PATH or a local node_modules).
//
// Fonts are loaded with the FontFace API from the exact bytes in fixtures/fonts
// (same files surimono registers), so no system font is involved. The canvas is
// detached, so relative sizes (em, %) resolve against canvas's 10px default.

import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require('playwright');
} catch {
  playwright = require('/opt/npm-tools/node_modules/playwright');
}

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../../..');
const fontsDir = resolve(process.argv[2] ?? join(repo, 'fixtures/fonts'));
const outFile = resolve(process.argv[3] ?? join(repo, 'crates/gfx/tests/data/chrome_measure_text.json'));

const FONTS = [
  { file: 'NotoSans-Regular.ttf', family: 'Noto Sans', weight: '400' },
  { file: 'NotoSans-Bold.ttf', family: 'Noto Sans', weight: '700' },
  { file: 'NotoSansJP-Regular.otf', family: 'Noto Sans JP', weight: '400' },
  { file: 'NotoSansJP-Bold.otf', family: 'Noto Sans JP', weight: '700' },
];

// `font` is what Chrome measures. `ours` is what surimono measures, when it
// differs: Chrome maps generic families and unknown names to system fonts, so
// for those cases Chrome gets the equivalent explicit registered family.
const CASES = [
  { text: 'Hello, world', font: "12px 'Noto Sans'" },
  { text: 'The quick brown fox jumps over the lazy dog', font: "16px 'Noto Sans'" },
  { text: 'Revenue 2026', font: "bold 14px 'Noto Sans'" },
  { text: 'AVATAR Wave Toyota', font: "13.5px 'Noto Sans'" },
  { text: 'office affine', font: "20px 'Noto Sans'" },
  { text: '0123456789', font: "11px 'Noto Sans'" },
  { text: '-12.5%', font: "600 12px 'Noto Sans'" },
  { text: 'gjpqy ÁÉÍ', font: "40px 'Noto Sans'" },
  { text: 'trailing  ', font: "12px 'Noto Sans'" },
  { text: '  leading', font: "12px 'Noto Sans'" },
  { text: 'a\tb\nc', font: "12px 'Noto Sans'" },
  { text: '', font: "12px 'Noto Sans'" },
  { text: 'x', font: "1px 'Noto Sans'" },
  { text: 'Wide label', font: "72px 'Noto Sans'" },
  { text: 'Points', font: "9pt 'Noto Sans'" },
  { text: 'Em size', font: "1.4em 'Noto Sans'" },
  { text: 'Percent size', font: "150% 'Noto Sans'" },
  { text: 'Line height', font: "italic 600 13.5px/1.2 'Noto Sans'" },
  { text: 'Generic', font: "12px 'Noto Sans'", ours: '12px sans-serif' },
  { text: 'Bold generic', font: "bold 12px 'Noto Sans'", ours: 'bold 12px sans-serif' },
  { text: 'Unknown family', font: "12px 'Noto Sans'", ours: "12px Arial, 'Noto Sans'" },
  { text: '売上高（百万円）', font: "12px 'Noto Sans JP'" },
  { text: '東京都品川区', font: "bold 16px 'Noto Sans JP'" },
  { text: 'ひらがなカタカナ', font: "14px 'Noto Sans JP'" },
  { text: 'Sales 売上 2026年', font: "12px 'Noto Sans', 'Noto Sans JP'" },
  { text: 'Sales 売上 2026年', font: "12px 'Noto Sans', 'Noto Sans JP'", ours: "12px 'Noto Sans'" },
  { text: '月別 Revenue', font: "bold 13px 'Noto Sans', 'Noto Sans JP'", ours: "bold 13px 'Noto Sans', sans-serif" },
  { text: 'Q1 Q2 Q3 Q4', font: "12px 'Noto Sans JP'" },
];

const sha = (b) => createHash('sha256').update(b).digest('hex');

const browser = await playwright.chromium.launch();
const version = browser.version();
const page = await browser.newPage();
await page.setContent('<!doctype html><meta charset="utf-8"><body></body>');

const fonts = FONTS.map((f) => {
  const bytes = readFileSync(join(fontsDir, f.file));
  return { ...f, bytes: bytes.length, sha256: sha(bytes), b64: bytes.toString('base64') };
});

const results = await page.evaluate(async ({ fonts, cases }) => {
  for (const f of fonts) {
    const bin = Uint8Array.from(atob(f.b64), (c) => c.charCodeAt(0));
    const face = new FontFace(f.family, bin.buffer, { weight: f.weight, style: 'normal' });
    await face.load();
    document.fonts.add(face);
  }
  await document.fonts.ready;
  const ctx = document.createElement('canvas').getContext('2d');
  const out = [];
  for (const c of cases) {
    ctx.font = '10px sans-serif';
    ctx.font = c.font;
    const m = ctx.measureText(c.text);
    out.push({
      text: c.text,
      font: c.font,
      ours: c.ours ?? c.font,
      resolved_font: ctx.font,
      width: m.width,
      actual_bounding_box_left: m.actualBoundingBoxLeft,
      actual_bounding_box_right: m.actualBoundingBoxRight,
      actual_bounding_box_ascent: m.actualBoundingBoxAscent,
      actual_bounding_box_descent: m.actualBoundingBoxDescent,
      font_bounding_box_ascent: m.fontBoundingBoxAscent,
      font_bounding_box_descent: m.fontBoundingBoxDescent,
    });
  }
  return out;
}, { fonts: fonts.map(({ b64, family, weight }) => ({ b64, family, weight })), cases: CASES });

await browser.close();

const doc = {
  generator: 'crates/gfx/tests/chrome/measure_text.mjs',
  chromium: version,
  playwright: require('playwright/package.json').version,
  fonts: fonts.map(({ file, family, weight, bytes, sha256 }) => ({ file, family, weight, bytes, sha256 })),
  cases: results,
};
mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, JSON.stringify(doc, null, 2) + '\n');
console.log(`wrote ${results.length} cases from Chromium ${version} to ${outFile}`);
