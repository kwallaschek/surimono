// Checks that sanitize_svg output is inert when inlined into HTML, using
// Chrome's own HTML parser: no script runs, nothing is fetched, and the parsed
// DOM has no script elements or event-handler attributes.
//
// Usage: cargo test -p surimono-gfx --test svg hostile -- --nocapture
//        node crates/gfx/tests/chrome/svg_inline.mjs <path printed by the test>
// As a control, the raw inputs are inlined too; those must trip the checks.

import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const require = createRequire(import.meta.url);
let playwright;
try { playwright = require('playwright'); } catch { playwright = require('/opt/npm-tools/node_modules/playwright'); }

const cases = JSON.parse(readFileSync(process.argv[2], 'utf8'));

const browser = await playwright.chromium.launch();

async function check(markup) {
  const page = await browser.newPage();
  const requests = [];
  const dialogs = [];
  page.on('dialog', async (d) => { dialogs.push(d.message()); await d.dismiss(); });
  await page.route('**/*', (route) => {
    const url = route.request().url();
    if (url === 'https://surimono.test/') {
      return route.fulfill({ contentType: 'text/html', body: markup });
    }
    requests.push(url);
    return route.abort();
  });
  await page.goto('https://surimono.test/');
  // Give onload, SMIL begin and lazy loads a moment, then hover everything.
  await page.waitForTimeout(300);
  await page.evaluate(() => {
    for (const el of document.querySelectorAll('*')) {
      for (const t of ['mouseover', 'click', 'focus']) el.dispatchEvent(new Event(t));
    }
  });
  await page.waitForTimeout(200);
  const dom = await page.evaluate(() => {
    const scripts = document.querySelectorAll('script').length;
    const handlers = [];
    for (const el of document.querySelectorAll('*')) {
      for (const a of el.attributes) if (/^on/i.test(a.name)) handlers.push(`${el.localName} ${a.name}`);
    }
    return { scripts, handlers, pwned: window.__pwned ?? 0 };
  });
  await page.close();
  return { requests, dialogs, ...dom };
}

const wrap = (svgs) => `<!doctype html><meta charset="utf-8"><script>window.__pwned=0;window.alert=()=>{window.__pwned++}</script><body>${svgs.join('\n')}</body>`;
const report = (r) => `alerts ${r.pwned + r.dialogs.length}, requests ${r.requests.length}, script elements ${r.scripts - 1}, handler attributes ${r.handlers.length}`;

const isClean = (r) => r.pwned + r.dialogs.length === 0 && r.requests.length === 0 && r.scripts === 1 && r.handlers.length === 0;

// All together, then each on its own page (so no case can hide another).
const together = await check(wrap(cases.map((c) => c.output)));
let alone = 0;
let rawTrips = 0;
for (const c of cases) {
  const r = await check(wrap([c.output]));
  if (isClean(r)) alone++;
  else console.log(`NOT CLEAN: ${c.input}\n  -> ${c.output}\n  ${report(r)}`);
  if (!isClean(await check(wrap([c.input])))) rawTrips++;
}
await browser.close();
console.log(`sanitized, one page (${cases.length} cases): ${report(together)}`);
console.log(`sanitized, one page per case: ${alone}/${cases.length} clean`);
console.log(`raw inputs, one page per case (control): ${rawTrips}/${cases.length} trip a check`);
const ok = isClean(together) && alone === cases.length && rawTrips > 0;
console.log(ok ? 'OK' : 'FAIL');
process.exit(ok ? 0 : 1);
