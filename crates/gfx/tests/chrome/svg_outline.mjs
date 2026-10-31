// Compares, in Chrome, island SVG whose <text> was outlined by surimono with
// the same SVG keeping its <text>, which Chrome draws with the same font files
// loaded through @font-face. Outside truth: Chrome's own text rendering.
//
// Usage (from the repo root):
//   cargo test -p surimono-gfx --test svg real_charts
//   node crates/gfx/tests/chrome/svg_outline.mjs [dir] [fonts-dir]
// dir defaults to $CARGO_TARGET_DIR/tmp/outlined (written by that test):
// pairs <name>.orig.svg (text kept) and <name>.svg (outlined).
//
// Score: of the pixels that are ink in either render (darker than 250 in any
// channel), the share that differ by more than 64 in some channel, at 2x
// device scale. Anti-aliasing of text and of paths differs slightly, so a
// few edge pixels always differ. Control: the original with its font stack
// replaced by system fonts (`serif`), the mismatch outlines exist to avoid.

import { createRequire } from 'node:module';
import { readFileSync, readdirSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let playwright;
try { playwright = require('playwright'); } catch { playwright = require('/opt/npm-tools/node_modules/playwright'); }

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../../..');
const dir = resolve(process.argv[2] ?? join(process.env.CARGO_TARGET_DIR ?? join(repo, 'target'), 'tmp/outlined'));
const fontsDir = resolve(process.argv[3] ?? join(repo, 'fixtures/fonts'));
const FONTS = [
  ['NotoSans-Regular.ttf', 'Noto Sans', 400], ['NotoSans-Bold.ttf', 'Noto Sans', 700],
  ['NotoSansJP-Regular.otf', 'Noto Sans JP', 400], ['NotoSansJP-Bold.otf', 'Noto Sans JP', 700],
];
// Pass threshold for the outlined render (share of ink pixels that differ).
const MAX_DIFF = 0.05;

const browser = await playwright.chromium.launch();
const context = await browser.newContext({ deviceScaleFactor: 2 });
const page = await context.newPage();
let current = '';
await page.route('**/*', (route) => {
  const url = new URL(route.request().url());
  if (url.pathname.startsWith('/fonts/')) {
    return route.fulfill({ body: readFileSync(join(fontsDir, url.pathname.slice(7))), contentType: 'font/otf' });
  }
  const faces = FONTS.map(([f, fam, w]) => `@font-face{font-family:'${fam}';src:url(/fonts/${f});font-weight:${w}}`).join('');
  return route.fulfill({ contentType: 'text/html', body: `<!doctype html><meta charset="utf-8"><style>${faces}body{margin:0;background:#fff}</style><body>${current}</body>` });
});

async function render(svg) {
  current = svg;
  await page.goto('https://outline.test/');
  await page.evaluate(async (fonts) => {
    for (const [, fam, w] of fonts) await document.fonts.load(`${w} 16px '${fam}'`, 'a売');
    await document.fonts.ready;
  }, FONTS);
  const box = await page.locator('svg').first().boundingBox();
  return (await page.screenshot({ clip: box })).toString('base64');
}

async function diff(a, b) {
  return page.evaluate(async ([a, b]) => {
    const load = async (s) => { const i = new Image(); i.src = `data:image/png;base64,${s}`; await i.decode(); return i; };
    const [ia, ib] = [await load(a), await load(b)];
    const w = Math.min(ia.width, ib.width), h = Math.min(ia.height, ib.height);
    const px = (img) => { const c = new OffscreenCanvas(w, h); const x = c.getContext('2d'); x.drawImage(img, 0, 0); return x.getImageData(0, 0, w, h).data; };
    const [da, db] = [px(ia), px(ib)];
    let ink = 0, bad = 0;
    for (let i = 0; i < da.length; i += 4) {
      const inkA = da[i] < 250 || da[i + 1] < 250 || da[i + 2] < 250;
      const inkB = db[i] < 250 || db[i + 1] < 250 || db[i + 2] < 250;
      if (!inkA && !inkB) continue;
      ink++;
      if (Math.max(Math.abs(da[i] - db[i]), Math.abs(da[i + 1] - db[i + 1]), Math.abs(da[i + 2] - db[i + 2])) > 64) bad++;
    }
    return { ink, bad, share: bad / Math.max(ink, 1), size: [w, h] };
  }, [a, b]);
}

let ok = true;
const names = readdirSync(dir).filter((f) => f.endsWith('.orig.svg')).map((f) => f.slice(0, -'.orig.svg'.length)).sort();
if (names.length === 0) { console.error(`no *.orig.svg in ${dir}`); process.exit(2); }
for (const name of names) {
  const orig = readFileSync(join(dir, `${name}.orig.svg`), 'utf8');
  const outlined = readFileSync(join(dir, `${name}.svg`), 'utf8');
  const control = orig.replace(/font-family:[^;"]*/g, 'font-family:serif')
    .replace(/font-family="[^"]*"/g, 'font-family="serif"');
  const [ro, rl, rc] = [await render(orig), await render(outlined), await render(control)];
  const d = await diff(ro, rl);
  const c = await diff(ro, rc);
  const pass = d.share <= MAX_DIFF && c.share > d.share * 3;
  ok &&= pass;
  console.log(`${name}: outlined vs Chrome text ${(d.share * 100).toFixed(2)}% of ${d.ink} ink px differ; ` +
    `control (system serif) ${(c.share * 100).toFixed(2)}%; ${pass ? 'ok' : 'FAIL'}`);
}
await browser.close();
console.log(ok ? 'OK' : 'FAIL');
process.exit(ok ? 0 : 1);
