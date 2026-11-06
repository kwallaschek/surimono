#!/usr/bin/env node
// Writes wpt/baseline/chrome.json: what the pinned Playwright Chromium reports
// for every URL in wpt/manifest.toml, at the commit slices.toml pins
// (wpt/README.md, "Baseline"). Generated, never edited.
//
//   node wpt/tools/chrome_baseline.mjs --from CHECKOUT [--filter STR] [--out FILE] [--update]
//
// CHECKOUT is a web-platform-tests checkout at the pinned commit. This script
// never runs anything from it on the host: a small static server here reads its
// files as data for Chromium, on 127.0.0.1. So it covers what WPT's server does
// statically (the .any/.window wrappers, .headers files, .sub
// substitution) and not its Python handlers, apart from two small ones redone
// below: another .py URL is a 404, and a subtest that needs one fails in Chromium
// and is left out of the denominator. The 404s are listed at the end.
//
// --update keeps the tests already in FILE and replaces only those --filter
// picks, in manifest order. It refuses a FILE from another Chromium or commit,
// so a baseline never mixes two browsers.

import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ALT_HOST, HOST, PORTS, substitute } from './substitute.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const wpt = path.resolve(here, '..');
const { chromium } = createRequire(path.join(wpt, '../fixtures/tools/package.json'))('playwright');

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const checkout = path.resolve(flag('--from') ?? '');
const filter = flag('--filter');
const out = flag('--out') ?? path.join(wpt, 'baseline/chrome.json');
const jobs = Number(flag('--jobs') ?? 2);
if (!flag('--from') || !fs.existsSync(path.join(checkout, 'resources/testharness.js'))) {
  console.error('usage: chrome_baseline.mjs --from WPT_CHECKOUT [--filter STR] [--out FILE] [--jobs N] [--update]');
  process.exit(1);
}

const ORIGIN = `http://${HOST}:${PORTS.http[0]}`;
// The harness's own timer stays off, as in the surimono runner; these are
// wptrunner's limits, on real time here.
const TIMEOUT_MS = 10_000;
const LONG_TIMEOUT_MS = 60_000;
const HOOK = `
setup({ output: false, explicit_timeout: true });
add_completion_callback((tests, status) => {
  self.__surimonoWptReport = JSON.stringify({
    status: status.status,
    message: status.message,
    tests: tests.map((t) => ({ name: t.name, status: t.status })),
  });
});
`;
const TYPES = {
  '.html': 'text/html', '.htm': 'text/html', '.xhtml': 'application/xhtml+xml', '.xht': 'application/xhtml+xml',
  '.xml': 'application/xml', '.svg': 'image/svg+xml', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.css': 'text/css', '.json': 'application/json', '.txt': 'text/plain', '.png': 'image/png', '.jpg': 'image/jpeg',
  '.gif': 'image/gif', '.idl': 'text/plain', '.woff2': 'font/woff2', '.ttf': 'font/ttf', '.webm': 'video/webm',
};
const missing = new Set();

/** A path under the checkout, or null: no escapes and no symlinks out. */
function inside(urlPath) {
  const file = path.join(checkout, decodeURIComponent(urlPath));
  if (!file.startsWith(checkout + path.sep)) return null;
  try {
    return fs.realpathSync(file).startsWith(fs.realpathSync(checkout) + path.sep) ? file : null;
  } catch {
    return null;
  }
}

/** The page WPT's server makes for x.any.html and x.window.html. */
function wrapper(urlPath, source) {
  const metas = [...source.matchAll(/^\/\/\s*META:\s*(\w+)=(.*)$/gm)];
  const lines = ['<!doctype html>', '<meta charset=utf-8>'];
  for (const [, key, value] of metas) {
    if (key === 'title') lines.push(`<title>${value.trim()}</title>`);
    if (key === 'timeout' && value.trim() === 'long') lines.push('<meta name=timeout content=long>');
  }
  lines.push('<script>self.GLOBAL = { isWindow() { return true; }, isWorker() { return false; }, isShadowRealm() { return false; } };</script>');
  lines.push('<script src="/resources/testharness.js"></script>', '<script src="/resources/testharnessreport.js"></script>');
  for (const [, key, value] of metas) {
    if (key === 'script') lines.push(`<script src="${value.trim()}"></script>`);
  }
  lines.push('<div id=log></div>', `<script src="${urlPath.replace(/\.html$/, '.js')}"></script>`);
  return lines.join('\n');
}

function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (c) => `&#${c.codePointAt(0)};`);
}

function serve(req, res) {
  const url = new URL(req.url, ORIGIN);
  const send = (code, type, body, headers = {}) => {
    res.writeHead(code, { 'Content-Type': type, ...headers });
    res.end(body);
  };
  if (url.pathname === '/resources/testharnessreport.js') return send(200, 'text/javascript', HOOK);
  if (url.pathname === '/dom/nodes/encoding.py') {
    // The one Python handler with real weight: two characterSet tests, 654 subtests.
    return send(200, 'text/html', `<!doctype html><meta charset="${escapeHtml(url.searchParams.get('label') ?? '')}">`);
  }
  if (url.pathname.endsWith('/Document-contentType/support/contenttype_setter.py')) {
    const type = url.searchParams.get('type');
    const subtype = url.searchParams.get('subtype');
    const mime = url.searchParams.get('mime');
    const meta = mime ? `<meta http-equiv="Content-Type" content="${mime}; charset=utf-8"/>` : '';
    return send(200, type && subtype ? `${type}/${subtype}` : 'text/html', `<head>${meta}</head>`);
  }
  const wrapped = /\.(any|window)\.html$/.test(url.pathname) ? inside(url.pathname.replace(/\.html$/, '.js')) : null;
  if (wrapped && fs.existsSync(wrapped)) return send(200, 'text/html', wrapper(url.pathname, fs.readFileSync(wrapped, 'utf8')));
  const file = inside(url.pathname);
  if (!file || file.endsWith('.py') || !fs.statSync(file).isFile()) {
    if (!url.pathname.endsWith('favicon.ico')) missing.add(url.pathname);
    return send(404, 'text/plain', 'not found');
  }
  const headers = {};
  if (fs.existsSync(`${file}.headers`)) {
    for (const line of fs.readFileSync(`${file}.headers`, 'utf8').split('\n')) {
      const i = line.indexOf(':');
      if (i > 0) headers[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
  }
  let body = fs.readFileSync(file);
  if (path.basename(file).includes('.sub.')) {
    try {
      body = substitute(body.toString('utf8'), url);
    } catch (e) {
      return send(500, 'text/plain', String(e)); // Where wptserve's template raises.
    }
  }
  const type = headers['Content-Type'] ?? TYPES[path.extname(file)] ?? 'application/octet-stream';
  delete headers['Content-Type'];
  send(200, type, body, headers);
}

const CODES = { harness: { 0: 'OK', 2: 'TIMEOUT', 3: 'PRECONDITION_FAILED' }, subtest: { 0: 'PASS', 2: 'TIMEOUT', 3: 'NOTRUN', 4: 'PRECONDITION_FAILED' } };

async function runTest(browser, url) {
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await context.newPage();
  const report = () => page.evaluate(() => globalThis.__surimonoWptReport ?? null);
  const waitReport = async (ms) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const text = await report().catch(() => null);
      if (text) return text;
      await page.waitForTimeout(25);
    }
    return null;
  };
  try {
    await page.goto(`${ORIGIN}/${url}`, { waitUntil: 'commit', timeout: 30_000 });
    const long = await page.evaluate(() => !!document.querySelector('meta[name=timeout][content=long]')).catch(() => false);
    let text = await waitReport(long ? LONG_TIMEOUT_MS : TIMEOUT_MS);
    if (!text) {
      await page.evaluate(() => { if (typeof timeout === 'function') timeout(); }).catch(() => {});
      text = await waitReport(2_000);
    }
    if (!text) return { status: 'TIMEOUT', subtests: [] };
    const r = JSON.parse(text);
    return {
      status: CODES.harness[r.status] ?? 'ERROR',
      subtests: r.tests.map((t) => ({ name: t.name, status: CODES.subtest[t.status] ?? 'FAIL' })),
    };
  } catch (e) {
    return { status: 'ERROR', subtests: [], message: String(e).split('\n')[0] };
  } finally {
    await context.close().catch(() => {});
  }
}

/** The tests in wpt/manifest.toml, read with the few rules vendor.py writes it by. */
function manifest() {
  const text = fs.readFileSync(path.join(wpt, 'manifest.toml'), 'utf8');
  const commit = /^commit = "(.*)"$/m.exec(text)[1];
  const tests = [];
  for (const block of text.split('\n\n')) {
    if (!block.startsWith('[[test]]')) continue;
    const field = (name) => new RegExp(`^${name} = (.*)$`, 'm').exec(block)[1];
    for (const u of JSON.parse(field('urls'))) tests.push({ slice: JSON.parse(field('slice')), test: u, file: JSON.parse(field('file')) });
  }
  return { commit, tests };
}

const { commit, tests } = manifest();
const todo = filter ? tests.filter((t) => t.test.includes(filter)) : tests;
const server = createServer(serve);
const server2 = createServer(serve);
await new Promise((ok) => server.listen(PORTS.http[0], '127.0.0.1', ok));
await new Promise((ok) => server2.listen(PORTS.http[1], '127.0.0.1', ok));
const browser = await chromium.launch({
  headless: true,
  args: [`--host-resolver-rules=MAP ${HOST} 127.0.0.1, MAP *.${HOST} 127.0.0.1, MAP ${ALT_HOST} 127.0.0.1, MAP *.${ALT_HOST} 127.0.0.1`, `--unsafely-treat-insecure-origin-as-secure=${ORIGIN}`],
});
const results = new Array(todo.length);
let next = 0;
await Promise.all(Array.from({ length: jobs }, async () => {
  while (next < todo.length) {
    const i = next++;
    const r = await runTest(browser, todo[i].test);
    results[i] = { ...todo[i], ...r };
    console.error(`[${i + 1}/${todo.length}] ${r.status} ${todo[i].test}`);
  }
}));
const version = browser.version();
await browser.close();
server.close();
server2.close();
server.closeAllConnections();
server2.closeAllConnections();
let written = results;
if (args.includes('--update')) {
  const prior = JSON.parse(fs.readFileSync(out, 'utf8'));
  if (prior.chromium !== version || prior.wpt_commit !== commit) {
    console.error(`--update: ${out} is from Chromium ${prior.chromium} at ${prior.wpt_commit}, this run is ${version} at ${commit}`);
    process.exit(1);
  }
  const fresh = new Map(results.map((r) => [r.test, r]));
  const old = new Map(prior.tests.map((r) => [r.test, r]));
  written = tests.map((t) => fresh.get(t.test) ?? old.get(t.test)).filter(Boolean);
}
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, `${JSON.stringify({ schema: 1, wpt_commit: commit, chromium: version, tests: written }, null, 0).replaceAll('},{"slice"', '},\n{"slice"')}\n`);
const count = (s) => results.filter((r) => r.status === s).length;
console.error(`${results.length} tests: ${count('OK')} OK, ${count('TIMEOUT')} TIMEOUT, ${count('ERROR')} ERROR; ${missing.size} URLs not served`);
for (const m of [...missing].sort()) console.error(`  404 ${m}`);
