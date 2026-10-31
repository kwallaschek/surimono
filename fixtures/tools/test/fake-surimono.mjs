#!/usr/bin/env node
// A stand-in for the surimono CLI, to dry-run checkpoint.mjs before the real
// pipeline is on main. It answers `render <in.html> -o <out.html> --font …` with
// what a perfect surimono would print: the fixture's prepass.html from
// `node reference.mjs --out DIR` (Chrome's own SVGs in surimono's wrapper).
//
//   FAKE_SURIMONO_FROM=DIR node checkpoint.mjs --out OUT --surimono fixtures/tools/test/fake-surimono.mjs
//
// FAKE_SURIMONO_FAIL=<fixture> makes that fixture's render exit 3 with a
// time-limit message, to check that a failed render is reported as failing.
// FAKE_SURIMONO_LEAK=TZ|LANG|cwd appends that value of the host to the output,
// a render that depends on its environment. Without FAKE_SURIMONO_FROM the
// output is a page with no islands (every comparison then fails without
// needing Chrome or fulgur), for checkpoint.mjs's own tests.
// Never a source of results: every score it produces is Chrome compared with
// Chrome.

import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
if (args[0] === "--version") {
  console.log("surimono 0.0.0 (fake-surimono.mjs: Chrome's SVGs, not surimono)");
  process.exit(0);
}
const out = args[args.indexOf("-o") + 1];
if (args[0] !== "render" || !args[1] || !out) {
  console.error("usage: fake-surimono.mjs render <in.html> -o <out.html> [--font <file>]...");
  process.exit(2);
}
const fixture = path.basename(path.dirname(path.resolve(args[1])));
if (process.env.FAKE_SURIMONO_FAIL === fixture) {
  console.error(`error: island 0 (echarts) hit the time limit (2000 ms)`);
  process.exit(3);
}
let html = "<!doctype html><title>no islands</title><p>fake-surimono</p>\n";
if (process.env.FAKE_SURIMONO_FROM) {
  const from = path.join(process.env.FAKE_SURIMONO_FROM, fixture, "prepass.html");
  if (!fs.existsSync(from)) {
    console.error(`no ${from}: set FAKE_SURIMONO_FROM to a reference.mjs --out directory`);
    process.exit(5);
  }
  html = fs.readFileSync(from, "utf8");
}
const leak = process.env.FAKE_SURIMONO_LEAK;
if (leak) html += `<!-- ${leak === "cwd" ? process.cwd() : process.env[leak]} -->\n`;
fs.writeFileSync(out, html);
