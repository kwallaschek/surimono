// Checks that sanitize_svg output is inert when inlined into HTML, using
// Chrome's own HTML parser: no script runs, nothing is fetched, the parsed
// DOM has no script elements or event-handler attributes, and the island
// stays in its box (wrapped like the facade does, the page around it keeps
// its shape and attributes, and no <style> or HTML element is inside).
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
    // Containment: the page keeps its shape, and islands hold SVG only.
    const breakouts = [];
    const attrs = (el) => [...el.attributes].map((a) => `${a.name}=${a.value}`).join(' ');
    if (attrs(document.documentElement) !== 'lang=en') breakouts.push(`html ${attrs(document.documentElement)}`);
    if (attrs(document.body) !== 'class=page') breakouts.push(`body ${attrs(document.body)}`);
    const after = document.getElementById('after');
    if (!after || after.parentNode !== document.body || document.body.lastElementChild !== after) breakouts.push('page after the islands moved');
    // Only the page's own utility sheet.
    if (document.querySelectorAll('style').length !== 1) breakouts.push('style element');
    for (const island of document.querySelectorAll('div.surimono-island')) {
      if (island.parentNode !== document.body) breakouts.push('island moved');
      for (const el of island.querySelectorAll('*')) {
        if (el.namespaceURI !== 'http://www.w3.org/2000/svg') breakouts.push(`<${el.localName}> in ${el.namespaceURI}`);
      }
      const root = island.firstElementChild;
      if (root) {
        const cs = getComputedStyle(root);
        if (cs.position !== 'static' || cs.transform !== 'none' || cs.float !== 'none' || cs.overflow !== 'hidden') {
          breakouts.push(`root ${cs.position} ${cs.transform} ${cs.float} ${cs.overflow}`);
        }
      }
    }
    return { scripts, handlers, breakouts, pwned: window.__pwned ?? 0 };
  });
  await page.close();
  return { requests, dialogs, ...dom };
}

// The host page has utility CSS like Tailwind's or Bootstrap's, which an
// island's root must not be able to pick up.
const HOST_CSS = '.fixed,.position-fixed,.fixed-top{position:fixed}.absolute{position:absolute}.inset-0{inset:0}.z-50{z-index:50}.float-left{float:left}.rotate-45{transform:rotate(45deg)}.overflow-visible{overflow:visible}[role=dialog],[aria-modal=true]{position:fixed}';
const wrap = (svgs) => `<!doctype html><html lang="en"><meta charset="utf-8"><style>${HOST_CSS}</style><script>window.__pwned=0;window.alert=()=>{window.__pwned++}</script><body class="page">${svgs.map((s) => `<div class="surimono-island">${s}</div>`).join('\n')}<p id="after">after</p></body></html>`;
const report = (r) => `alerts ${r.pwned + r.dialogs.length}, requests ${r.requests.length}, script elements ${r.scripts - 1}, handler attributes ${r.handlers.length}, breakouts ${r.breakouts.length}${r.breakouts.length ? ` (${r.breakouts.slice(0, 3).join('; ')})` : ''}`;

const isClean = (r) => r.pwned + r.dialogs.length === 0 && r.requests.length === 0 && r.scripts === 1 && r.handlers.length === 0 && r.breakouts.length === 0;

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
