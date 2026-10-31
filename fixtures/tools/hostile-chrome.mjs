#!/usr/bin/env node
// Runs the hostile-script suite in the pinned Chromium, so every "Chromium 153:
// …" claim in fixtures/hostile/manifest.toml can be checked by anyone.
//
//   node fixtures/tools/hostile-chrome.mjs [--timeout-ms 4000] [--json FILE] [case-id ...]
//
// Each case (each step of a `steps` case) runs alone in a fresh dedicated
// Worker: the file's text through indirect eval as a classic script, the
// result awaited if it is a Promise, the Worker terminated after the timeout.
// The page is served from a virtual origin and every other request is
// aborted, so nothing reaches the network. Prints one row per case:
//
//   Ok "<result>"            the script finished with a string
//   Ok <type> [value]        it finished with something else (surimono: Exception)
//   <ErrorName> "<message>"  it threw, or its promise rejected
//   still running after N s  the Worker was terminated
//
// Chrome is outside truth for what a browser does, not the expectation itself:
// the manifest's `expect` follows the spec and the safety rule; a claim in its
// about text must match this output.

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, TOOLS_DIR, launchChromium } from "./lib/fixtures.mjs";

const require = createRequire(path.join(TOOLS_DIR, "package.json"));
const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { "timeout-ms": { type: "string", default: "4000" }, json: { type: "string" } },
});
const timeoutMs = Number(values["timeout-ms"]);
const HOSTILE = path.join(FIXTURES_DIR, "hostile");

/** The cases of manifest.toml as [{ id, step, file }] (a minimal reader: id, file and steps' files). */
function cases() {
  const text = fs.readFileSync(path.join(HOSTILE, "manifest.toml"), "utf8");
  const out = [];
  for (const block of text.split(/^\[\[case\]\]\s*$/m).slice(1)) {
    const id = /^id\s*=\s*"([^"]+)"/m.exec(block)[1];
    const file = /^file\s*=\s*"([^"]+)"/m.exec(block);
    if (file) {
      out.push({ id, step: null, file: file[1] });
      continue;
    }
    const steps = /^steps\s*=\s*\[([\s\S]*?)\]\s*$/m.exec(block);
    [...(steps?.[1] ?? "").matchAll(/file\s*=\s*"([^"]+)"/g)].forEach((m, i) => out.push({ id, step: i + 1, file: m[1] }));
  }
  return out;
}

// Runs in the page: one Worker per case, raced against the timeout.
async function runInWorker([source, timeout]) {
  const worker = `onmessage = async (e) => {
    try {
      let v = (0, eval)(e.data);
      if (v instanceof Promise) v = await v;
      postMessage(typeof v === "string" ? { kind: "ok", value: v } : { kind: "ok-other", type: v === null ? "null" : typeof v, value: ["number", "boolean", "bigint"].includes(typeof v) ? String(v) : undefined });
    } catch (err) {
      postMessage({ kind: "error", name: (err && err.name) || typeof err, message: String((err && err.message) || err) });
    }
  };`;
  const w = new Worker(URL.createObjectURL(new Blob([worker], { type: "text/javascript" })));
  const t0 = performance.now();
  const result = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ kind: "timeout" }), timeout);
    w.onmessage = (e) => {
      clearTimeout(timer);
      resolve(e.data);
    };
    w.onerror = (e) => {
      clearTimeout(timer);
      e.preventDefault();
      resolve({ kind: "error", name: "Error", message: e.message });
    };
    w.postMessage(source);
  });
  w.terminate();
  return { ...result, ms: Math.round(performance.now() - t0) };
}

const describe = (r, timeout) =>
  r.kind === "ok"
    ? `Ok ${JSON.stringify(r.value.length > 60 ? `${r.value.slice(0, 60)}…` : r.value)}`
    : r.kind === "ok-other"
      ? `Ok ${r.type}${r.value === undefined ? "" : ` ${r.value}`} (not a string)`
      : r.kind === "timeout"
        ? `still running after ${timeout / 1000} s`
        : `${r.name} ${JSON.stringify(r.message.slice(0, 80))}`;

const browser = await launchChromium();
const browserVersion = browser.version();
const rows = [];
try {
  const context = await browser.newContext();
  const ORIGIN = "https://hostile.surimono.test/";
  await context.route("**/*", (route) =>
    route.request().url() === ORIGIN ? route.fulfill({ contentType: "text/html", body: "<!doctype html><title>hostile</title>" }) : route.abort(),
  );
  const page = await context.newPage();
  await page.goto(ORIGIN);
  for (const c of cases().filter((c) => !positionals.length || positionals.includes(c.id))) {
    const source = fs.readFileSync(path.join(HOSTILE, c.file), "utf8");
    const r = await page.evaluate(runInWorker, [source, timeoutMs]);
    const name = c.step === null ? c.id : `${c.id} step ${c.step}`;
    rows.push({ ...c, result: r, text: describe(r, timeoutMs) });
    console.log(`${name.padEnd(40)} ${describe(r, timeoutMs)}${r.kind === "timeout" ? "" : ` (${r.ms} ms)`}`);
  }
} finally {
  await browser.close();
}
const playwright = JSON.parse(fs.readFileSync(require.resolve("playwright/package.json"), "utf8")).version;
console.log(`\n${rows.length} cases in Chromium ${browserVersion} (Playwright ${playwright}), ${timeoutMs} ms per case`);
if (values.json) fs.writeFileSync(values.json, `${JSON.stringify({ chromium: browserVersion, playwright, timeout_ms: timeoutMs, cases: rows }, null, 2)}\n`);
