#!/usr/bin/env node
// Checks which of results.json's Chromium answers depend on how run.mjs opens a
// page and starts the browser, for the cases about input (docs/COMPAT.md, "Out of
// scope: user input"). Prints; writes nothing.
//
//   node wpt/probes/first-load.mjs [--no-blink-settings] [CASE...]
//
// run.mjs opens a tab with Playwright's newPage() and then goto(), and Chromium,
// when a tab's first page replaces about:blank, synthesizes one trusted mouse move
// at the viewport's origin after load. Here each case is its tab's first load,
// created at the URL over the DevTools protocol (Target.createTarget), where no
// mouse event arrives. --no-blink-settings also starts Chromium without the flag
// Playwright sets what matchMedia says of hover and pointer with.

import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cases, page } from './cases.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(path.join(here, '../../fixtures/tools/package.json'))('playwright');
const args = process.argv.slice(2);
const BLINK = '--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4';
const noBlink = args.includes('--no-blink-settings');
const ids = args.filter((a) => !a.startsWith('--'));
const todo = ids.length ? ids : ['focus', 'autofocus', 'hover', 'scroll', 'mqhover', 'inputev'];
const committed = JSON.parse(fs.readFileSync(path.join(here, 'results.json'), 'utf8')).cases;
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

const server = createServer((req, res) => {
  const body = cases[req.url.slice(1)];
  res.writeHead(body ? 200 : 404, { 'Content-Type': 'text/html' });
  res.end(body ? page(body) : '');
});
await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
const base = `http://127.0.0.1:${server.address().port}`;
// Playwright starts the browser, so the build and the other flags are run.mjs's;
// the pages are opened through a browser-level DevTools session, not its page API.
const browser = await chromium.launch({ ignoreDefaultArgs: noBlink ? [BLINK] : [] });
const cdp = await browser.newBrowserCDPSession();
// One session per tab: flat sessions aren't reachable through Playwright's
// CDPSession, so each tab is driven by Target.sendMessageToTarget.
let next = 0;
const replies = new Map();
cdp.on('Target.receivedMessageFromTarget', ({ message }) => {
  const m = JSON.parse(message);
  if (m.id && replies.has(m.id)) replies.get(m.id)(m.result);
});
const evaluate = (sessionId, expression) => new Promise((ok) => {
  const id = ++next;
  replies.set(id, (r) => ok(r?.result?.value));
  cdp.send('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }) });
});
let same = 0;
for (const id of todo) {
  const { browserContextId } = await cdp.send('Target.createBrowserContext');
  const { targetId } = await cdp.send('Target.createTarget', { url: `${base}/${id}`, browserContextId });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: false });
  for (let i = 0; i < 200 && (await evaluate(sessionId, 'document.readyState')) !== 'complete'; i++) await sleep(20);
  await sleep(300);
  const log = JSON.parse(await evaluate(sessionId, 'JSON.stringify(L)'));
  await cdp.send('Target.disposeBrowserContext', { browserContextId });
  const want = committed[id]?.chrome.log;
  if (JSON.stringify(log) === JSON.stringify(want)) {
    same++;
    console.log(`${id}: the same as results.json`);
  } else {
    console.log(`${id}: differs\n  first load   ${JSON.stringify(log)}\n  results.json ${JSON.stringify(want)}`);
  }
}
console.log(`${browser.version()}${noBlink ? ', without ' + BLINK : ''}: ${todo.length} cases as a tab's first load, ${same} log what results.json has for Chromium`);
await browser.close();
server.close();
