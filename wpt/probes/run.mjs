#!/usr/bin/env node
// Writes wpt/probes/results.json: what the pinned Chromium and surimono log for
// each page in cases.mjs.
//
//   node wpt/probes/run.mjs RUNNER [--filter STR]
//
// RUNNER is the surimono-wpt-runner binary (wpt/README.md, "Runner"), which runs
// one page with --probe. Pages are served from memory; no network.

import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cases, page } from './cases.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(path.join(here, '../../fixtures/tools/package.json'))('playwright');
const [runner] = process.argv.slice(2);
if (!runner) {
  console.error('usage: run.mjs RUNNER');
  process.exit(1);
}
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wpt-probes-'));
const browser = await chromium.launch();
const results = {};
for (const [id, body] of Object.entries(cases)) {
  const html = page(body);
  const tab = await browser.newPage();
  const pageErrors = [];
  tab.on('pageerror', (e) => pageErrors.push(String(e)));
  await tab.route('http://probe.test/**', (r) => r.fulfill({ contentType: 'text/html', body: html }));
  await tab.goto(`http://probe.test/${id}`, { waitUntil: 'load' });
  await tab.waitForTimeout(50);
  const chrome = { log: await tab.evaluate(() => L), pageErrors };
  await tab.close();
  const file = path.join(tmp, `${id}.html`);
  fs.writeFileSync(file, html);
  const surimono = JSON.parse(execFileSync(runner, ['--probe', file], { encoding: 'utf8', maxBuffer: 1 << 24 }));
  fs.rmSync(file);
  results[id] = { chrome, surimono };
}
const version = browser.version();
await browser.close();
fs.rmdirSync(tmp);
fs.writeFileSync(path.join(here, 'results.json'), `${JSON.stringify({ chromium: version, cases: results }, null, 1)}\n`);
const same = Object.values(results).filter((r) => JSON.stringify(r.chrome.log) === JSON.stringify(r.surimono.log)).length;
console.error(`${Object.keys(results).length} cases, ${same} log the same in both`);
