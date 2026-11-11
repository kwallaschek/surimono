#!/usr/bin/env node
// A stand-in for surimono-page-check's `pages` mode, to test print-pages.mjs and
// printers.mjs without building the library. It answers
//   pages --root REPO --out DIR --run NAME [fixture ...]
// with what a perfect surimono would give: each page's reference/dom.html, Chrome's
// own DOM, twice, and a report. Never a source of results.

import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const flag = (name) => args[args.indexOf(name) + 1];
if (args[0] !== "pages" || !args.includes("--root") || !args.includes("--out") || !args.includes("--run")) {
  console.error("usage: fake-page-check.mjs pages --root REPO --out DIR --run NAME [fixture ...]");
  process.exit(2);
}
const names = [];
for (let i = 1; i < args.length; i++) {
  if (args[i].startsWith("--")) i++;
  else names.push(args[i]);
}
const suite = path.join(flag("--root"), "fixtures", "real-world");
for (const name of fs.readdirSync(suite).sort()) {
  const dom = path.join(suite, name, "reference", "dom.html");
  if (!fs.existsSync(path.join(suite, name, "input.html")) || (names.length && !names.includes(name))) continue;
  const target = path.join(flag("--out"), name);
  fs.mkdirSync(target, { recursive: true });
  const html = fs.readFileSync(dom);
  for (const pass of [1, 2]) fs.writeFileSync(path.join(target, `${flag("--run")}-${pass}.html`), html);
  const report = { settled: "Idle", uncaught: [], errors: [], unhandled_rejections: [], failed_count: 0, bytes: html.length, ms: 0 };
  fs.writeFileSync(path.join(target, `${flag("--run")}.json`), JSON.stringify([report, report]));
}
