#!/usr/bin/env node
// Renders the same island specs in Chromium with the libraries' official
// builds and compares the result with surimono's SVGs.
//
//   node chrome-check.js <renderer> <specs-dir> <surimono-svg-dir> [width height]
//
// <renderer> is echarts, vega or vega-lite. <specs-dir> holds <name>.json specs, <surimono-svg-dir>
// the matching <name>.svg files written by the islands tests with
// SURIMONO_DUMP_DIR set. Three Chrome renders per spec:
//
//   same-metrics   the library's server-side path in Chrome (ECharts SSR,
//                  Vega's headless view.toSVG()), with text measured by
//                  the same deterministic formula as surimono's font-free
//                  stand-in (TextMeasurer::approximate). Isolates the engine
//                  and the bundle: QuickJS + our bundle vs V8 + the official
//                  build. Expected to be byte-identical.
//   chrome-ssr     the same server-side path with Chrome's own canvas text
//                  metrics.
//   chrome-dom     the chart mounted in a <div> with the SVG renderer, as a
//                  browser-printed page (Grover, Ferrum) would draw it.
//
// Vega specs set their own size; the width and height only apply to ECharts.
//
// For each it prints whether the SVG is identical, and otherwise the
// structural differences: element counts by tag and the multiset of text
// contents. Pixel comparison is the conformance suite's job (fixtures/).
//
// Needs Playwright with its Chromium (require('playwright')); it never
// downloads browsers. Exit code 1 if a same-metrics render is not identical.
'use strict';

const fs = require('fs');
const path = require('path');

let playwright;
try {
  playwright = require('playwright');
} catch (e) {
  console.error('chrome-check: needs the playwright package with Chromium installed');
  process.exit(2);
}

const [renderer, specsDir, svgDir, w = '600', h = '400'] = process.argv.slice(2);
if (!renderer || !specsDir || !svgDir) {
  console.error('usage: node chrome-check.js <renderer> <specs-dir> <surimono-svg-dir> [width height]');
  process.exit(2);
}
const width = Number(w);
const height = Number(h);

const LIBS = {
  echarts: ['echarts/dist/echarts.min.js'],
  vega: ['vega/build/vega.min.js'],
  'vega-lite': ['vega/build/vega.min.js', 'vega-lite/build/vega-lite.min.js'],
};
if (!LIBS[renderer]) {
  console.error(`chrome-check: unknown renderer ${renderer}`);
  process.exit(2);
}

// Runs in the page. Mirrors crates/islands/src/measure.rs `approximate`.
function installApproximateMeasurer() {
  function fontSizePx(font) {
    for (const token of font.split(/\s+/)) {
      if (!token) continue;
      const size = token.split('/')[0];
      let n;
      let f;
      if (size.endsWith('px')) [n, f] = [size.slice(0, -2), 1];
      else if (size.endsWith('pt')) [n, f] = [size.slice(0, -2), 4 / 3];
      else if (size.endsWith('rem')) [n, f] = [size.slice(0, -3), 16];
      else if (size.endsWith('em')) [n, f] = [size.slice(0, -2), 16];
      else if (size.endsWith('%')) [n, f] = [size.slice(0, -1), 0.16];
      else continue;
      const v = Number(n);
      if (n !== '' && Number.isFinite(v) && v >= 0) return v * f;
    }
    return 10;
  }
  function isWide(cp) {
    return (cp >= 0x1100 && cp <= 0x115f) || (cp >= 0x2e80 && cp <= 0xa4cf) ||
      (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xfe30 && cp <= 0xfe4f) || (cp >= 0xff00 && cp <= 0xff60) ||
      (cp >= 0xffe0 && cp <= 0xffe6) || (cp >= 0x1f300 && cp <= 0x1faff) ||
      (cp >= 0x20000 && cp <= 0x3fffd);
  }
  window.__approximate = function (text, font) {
    const em = fontSizePx(font || '10px sans-serif');
    let wide = 0;
    let narrow = 0;
    for (const c of String(text)) {
      if (isWide(c.codePointAt(0))) wide++;
      else narrow++;
    }
    const width = 3 * narrow * em / 5 + wide * em;
    return {
      width,
      actualBoundingBoxLeft: 0,
      actualBoundingBoxRight: width,
      actualBoundingBoxAscent: 4 * em / 5,
      actualBoundingBoxDescent: em / 5,
      fontBoundingBoxAscent: 9 * em / 10,
      fontBoundingBoxDescent: em / 4,
    };
  };
}

// Runs in the page: the render contracts, as in crates/islands/js/src/.
async function renderInPage({ renderer, mode, specJson, width, height }) {
  const spec = JSON.parse(specJson);
  if (renderer === 'echarts') {
    const off = (o) => {
      if (!o || typeof o !== 'object' || Array.isArray(o)) return;
      o.animation = false;
      const s = o.series;
      if (Array.isArray(s)) s.forEach((x) => x && typeof x === 'object' && (x.animation = false));
      else if (s && typeof s === 'object') s.animation = false;
    };
    off(spec);
    off(spec.baseOption);
    if (Array.isArray(spec.options)) spec.options.forEach(off);
    if (mode === 'chrome-dom') {
      const div = document.createElement('div');
      div.style.width = width + 'px';
      div.style.height = height + 'px';
      document.body.appendChild(div);
      const chart = echarts.init(div, null, { renderer: 'svg', width, height, locale: 'EN' });
      try {
        chart.setOption(spec);
        return div.querySelector('svg').outerHTML;
      } finally {
        chart.dispose();
        div.remove();
      }
    }
    if (mode === 'same-metrics') {
      echarts.setPlatformAPI({ measureText: (t, f) => window.__approximate(t, f) });
    }
    const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width, height, locale: 'EN' });
    try {
      chart.setOption(spec);
      return chart.renderToSVGString();
    } finally {
      chart.dispose();
    }
  }
  if (renderer === 'vega' || renderer === 'vega-lite') {
    const vgSpec = renderer === 'vega-lite' ? vegaLite.compile(spec).spec : spec;
    if (mode === 'same-metrics') {
      // Vega measures with its own 2D context; swap the metrics underneath.
      CanvasRenderingContext2D.prototype.measureText = function (text) {
        return window.__approximate(text, this.font);
      };
    }
    if (mode === 'chrome-dom') {
      const div = document.createElement('div');
      document.body.appendChild(div);
      const view = new vega.View(vega.parse(vgSpec), { renderer: 'svg', container: div });
      try {
        await view.runAsync();
        return div.querySelector('svg').outerHTML;
      } finally {
        view.finalize();
        div.remove();
      }
    }
    const view = new vega.View(vega.parse(vgSpec), { renderer: 'none' });
    try {
      await view.runAsync();
      return await view.toSVG();
    } finally {
      view.finalize();
    }
  }
  throw new Error('unknown renderer ' + renderer);
}

// Runs in the page: element counts by tag and the texts, in document order.
function summarize(svg) {
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
  const err = doc.querySelector('parsererror');
  if (err) return { error: err.textContent };
  const tags = {};
  const texts = [];
  for (const el of doc.documentElement.querySelectorAll('*')) {
    tags[el.localName] = (tags[el.localName] || 0) + 1;
    if (el.localName === 'text') texts.push(el.textContent);
  }
  return { tags, texts };
}

function multisetDiff(a, b) {
  const count = new Map();
  for (const x of a) count.set(x, (count.get(x) || 0) + 1);
  for (const x of b) count.set(x, (count.get(x) || 0) - 1);
  const onlyA = [];
  const onlyB = [];
  for (const [x, n] of count) {
    for (let i = 0; i < n; i++) onlyA.push(x);
    for (let i = 0; i < -n; i++) onlyB.push(x);
  }
  return { onlyA, onlyB };
}

function compare(ours, theirs) {
  const lines = [];
  if (ours.error || theirs.error) {
    return [`parse error: ours ${ours.error || 'ok'}, chrome ${theirs.error || 'ok'}`];
  }
  const tags = new Set([...Object.keys(ours.tags), ...Object.keys(theirs.tags)]);
  for (const t of [...tags].sort()) {
    const a = ours.tags[t] || 0;
    const b = theirs.tags[t] || 0;
    if (a !== b) lines.push(`<${t}>: surimono ${a}, chrome ${b}`);
  }
  const { onlyA, onlyB } = multisetDiff(ours.texts, theirs.texts);
  if (onlyA.length) lines.push(`texts only in surimono: ${JSON.stringify(onlyA)}`);
  if (onlyB.length) lines.push(`texts only in chrome: ${JSON.stringify(onlyB)}`);
  if (!onlyA.length && !onlyB.length && ours.texts.join('\u0000') !== theirs.texts.join('\u0000')) {
    lines.push('same texts in a different order');
  }
  return lines;
}

function firstDifference(a, b) {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return `first difference at UTF-16 unit ${i}: surimono …${JSON.stringify(a.slice(Math.max(0, i - 40), i + 60))}… chrome …${JSON.stringify(b.slice(Math.max(0, i - 40), i + 60))}…`;
}

(async () => {
  const browser = await playwright.chromium.launch();
  // surimono renders with local time = UTC unless told otherwise.
  const context = await browser.newContext({ timezoneId: 'UTC', locale: 'en-US' });
  const libPaths = LIBS[renderer].map((p) => require.resolve(path.join(__dirname, 'node_modules', p)));
  const names = fs.readdirSync(specsDir).filter((f) => f.endsWith('.json')).map((f) => f.slice(0, -5)).sort();
  let failed = false;
  for (const name of names) {
    const specJson = fs.readFileSync(path.join(specsDir, name + '.json'), 'utf8');
    const svgPath = path.join(svgDir, name + '.svg');
    if (!fs.existsSync(svgPath)) {
      console.log(`${name}: no surimono SVG at ${svgPath}, skipped`);
      continue;
    }
    const ours = fs.readFileSync(svgPath, 'utf8');
    for (const mode of ['same-metrics', 'chrome-ssr', 'chrome-dom']) {
      // A fresh page per render: the platform API is global state.
      const page = await context.newPage();
      await page.setContent('<!doctype html><html><body style="margin:0"></body></html>');
      for (const p of libPaths) await page.addScriptTag({ path: p });
      await page.evaluate(installApproximateMeasurer);
      const theirs = await page.evaluate(renderInPage, { renderer, mode, specJson, width, height });
      const a = await page.evaluate(summarize, ours);
      const b = await page.evaluate(summarize, theirs);
      await page.close();
      if (ours === theirs) {
        console.log(`${name} [${mode}]: identical (${ours.length} UTF-16 units)`);
        continue;
      }
      const diffs = compare(a, b);
      if (mode === 'same-metrics') failed = true;
      console.log(`${name} [${mode}]: ${diffs.length ? 'structure differs' : 'same structure, different bytes'}` +
        ` (surimono ${ours.length}, chrome ${theirs.length} UTF-16 units)`);
      for (const d of diffs) console.log(`    ${d}`);
      if (mode === 'same-metrics') console.log(`    ${firstDifference(ours, theirs)}`);
    }
  }
  await browser.close();
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(2);
});
