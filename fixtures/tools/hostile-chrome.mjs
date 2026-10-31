#!/usr/bin/env node
// Runs the hostile-script suite in the pinned Chromium, so every "Chromium 153:
// …" claim in fixtures/hostile/manifest.toml can be checked by anyone.
//
//   node fixtures/tools/hostile-chrome.mjs [--timeout-ms 4000] [--retry-factor 3] [--json FILE] [case-id ...]
//
// Each case (each step of a `steps` case) runs alone in a fresh dedicated
// Worker: the file's text through indirect eval as a classic script, the
// result awaited if it is a Promise, the Worker terminated after the timeout.
// A case that times out runs once more with --retry-factor times the timeout
// (load-tolerant: only a case still running then is reported as running).
// The page is served from a virtual origin and every other request, and every
// WebSocket, is refused, so nothing reaches the network. Prints one row per case:
//
//   Ok "<result>"            the script finished with a string
//   Ok <type> [value]        it finished with something else (surimono: Exception)
//   <ErrorName> "<message>"  it threw, or its promise rejected
//   still running after N s  the Worker was terminated, twice (N is the timeout
//                            of the second, longer run)
//
// Chrome is outside truth for what a browser does, not the expectation itself:
// the manifest's `expect` follows the spec and the safety rule; a claim in its
// about text must match this output.

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { parseArgs } from "node:util";
import { TOOLS_DIR, launchChromium } from "./lib/fixtures.mjs";
import { HOSTILE_DIR as HOSTILE, hostileCases as cases } from "./lib/hostile.mjs";

const require = createRequire(path.join(TOOLS_DIR, "package.json"));
const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { "timeout-ms": { type: "string", default: "4000" }, "retry-factor": { type: "string", default: "3" }, json: { type: "string" } },
});
const timeoutMs = Number(values["timeout-ms"]);
// A case that times out is run again with a longer timeout before it is called
// "still running", so a loaded machine can't turn a slow finish into a hang.
const retryMs = timeoutMs * Number(values["retry-factor"]);

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
  await context.routeWebSocket(/.*/, (ws) => ws.close({ code: 1008, reason: "no network in the hostile suite" }));
  const page = await context.newPage();
  await page.goto(ORIGIN);
  for (const c of cases().filter((c) => !positionals.length || positionals.includes(c.id))) {
    const source = fs.readFileSync(path.join(HOSTILE, c.file), "utf8");
    let r = await page.evaluate(runInWorker, [source, timeoutMs]);
    let limit = timeoutMs;
    if (r.kind === "timeout" && retryMs > timeoutMs) {
      limit = retryMs;
      r = { ...(await page.evaluate(runInWorker, [source, retryMs])), retried: true };
    }
    const name = c.step === null ? c.id : `${c.id} step ${c.step}`;
    const text = describe(r, limit) + (r.retried && r.kind !== "timeout" ? ` (only on the ${limit / 1000} s retry: slower than ${timeoutMs / 1000} s here)` : "");
    rows.push({ ...c, result: r, text });
    console.log(`${name.padEnd(40)} ${text}${r.kind === "timeout" ? "" : ` (${r.ms} ms)`}`);
  }
} finally {
  await browser.close();
}
const playwright = JSON.parse(fs.readFileSync(require.resolve("playwright/package.json"), "utf8")).version;
console.log(`\n${rows.length} cases in Chromium ${browserVersion} (Playwright ${playwright}), ${timeoutMs} ms per case, ${retryMs} ms on retry`);
if (values.json) fs.writeFileSync(values.json, `${JSON.stringify({ chromium: browserVersion, playwright, timeout_ms: timeoutMs, retry_ms: retryMs, cases: rows }, null, 2)}\n`);
