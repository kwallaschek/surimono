// Reads fixtures/hostile/manifest.toml for the Node tools. A minimal reader
// of the few keys they need (id, file, and the files of `steps`), checked
// against Python's tomllib by test/hostile.test.mjs.

import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { FIXTURES_DIR } from "./fixtures.mjs";

export const HOSTILE_DIR = path.join(FIXTURES_DIR, "hostile");

/** The cases of a manifest's text as [{ id, step, file }], one entry per step of a `steps` case (step 1, 2, ...; null otherwise). */
export function hostileCases(text = fs.readFileSync(path.join(HOSTILE_DIR, "manifest.toml"), "utf8")) {
  const out = [];
  for (const block of text.split(/^\[\[case\]\]\s*$/m).slice(1)) {
    const id = /^id\s*=\s*"([^"]+)"/m.exec(block)?.[1];
    if (!id) throw new Error(`a [[case]] without an id: ${block.slice(0, 80)}`);
    const file = /^file\s*=\s*"([^"]+)"/m.exec(block);
    if (file) {
      out.push({ id, step: null, file: file[1] });
      continue;
    }
    const steps = /^steps\s*=\s*\[([\s\S]*?)\]\s*$/m.exec(block);
    const files = [...(steps?.[1] ?? "").matchAll(/file\s*=\s*"([^"]+)"/g)].map((m) => m[1]);
    if (!files.length) throw new Error(`case ${id} has neither file nor steps`);
    files.forEach((f, i) => out.push({ id, step: i + 1, file: f }));
  }
  return out;
}

/**
 * Playwright's own --disable-features list for Chromium, read from the
 * installed playwright-core. Chromium keeps only the last --disable-features
 * switch, so the suite's switch has to repeat Playwright's list or it would
 * switch those features back on (HttpsUpgrades, PaintHolding, ...).
 */
function playwrightDisabledFeatures(require) {
  const core = createRequire(require.resolve("playwright"));
  const bundle = fs.readFileSync(core.resolve("playwright-core/lib/coreBundle"), "utf8");
  const list = /\bdisabledFeatures = \[([\s\S]*?)\]/.exec(bundle)?.[1] ?? "";
  const names = [...list.replace(/\/\/[^\n]*/g, "").matchAll(/"([A-Za-z0-9_]+)"/g)].map((m) => m[1]);
  if (!names.includes("HttpsUpgrades")) {
    throw new Error("can't find Playwright's default --disable-features list in playwright-core/lib/coreBundle; update lib/hostile.mjs for this Playwright version");
  }
  return names;
}

/**
 * Chromium's launch options for the hostile suite:
 * - every connection goes through a proxy that doesn't exist (127.0.0.1:1),
 *   loopback too (`<-loopback>` removes the implicit bypass);
 * - IsolateSandboxedIframes is off, so a sandboxed iframe stays in its
 *   parent's renderer, where the init script reaches it before its own
 *   scripts run (in a process of its own, a sandboxed srcdoc iframe ran first
 *   and kept WebRTC). Playwright's own disabled features are kept.
 * Needs Playwright; returns a fresh object each call.
 */
export function offlineLaunch() {
  const require = createRequire(import.meta.url);
  const disabled = [...playwrightDisabledFeatures(require), "IsolateSandboxedIframes"];
  return {
    channel: "chromium",
    proxy: { server: "http://127.0.0.1:1" },
    args: ["--proxy-bypass-list=<-loopback>", `--disable-features=${disabled.join(",")}`],
  };
}

/**
 * An init script that deletes WebRTC (RTCPeerConnection and every other
 * RTC* interface, with the webkit aliases) from a frame before any of its
 * scripts runs. WebRTC sends STUN over UDP, which bypasses the proxy.
 */
export function removeWebRtc() {
  for (const name of Object.getOwnPropertyNames(globalThis)) {
    if (/^(webkit)?RTC/.test(name)) delete globalThis[name];
  }
}

/**
 * A page of the pinned Chromium (channel "chromium", as the references use)
 * from which nothing reaches the network, from the page itself or its Workers:
 * - Chromium sends every connection through a proxy that doesn't exist
 *   (offlineLaunch). That covers dedicated Workers too: Playwright's
 *   routeWebSocket doesn't see their WebSockets, and a review probe made a
 *   real TCP connection without the proxy;
 * - WebRTC, whose STUN packets go over UDP past the proxy, is deleted from
 *   every frame of the page before any script runs (removeWebRtc): the page,
 *   its iframes, sandboxed ones included (offlineLaunch keeps them in the
 *   page's process), and windows it opens. Dedicated Workers don't have it;
 * - Playwright's routes refuse every request but the page itself and close
 *   every WebSocket they do see.
 * test/hostile.test.mjs checks the TCP path from a Worker, and the UDP path
 * from the page, an iframe, a sandboxed srcdoc iframe and a window.open
 * window, against local listeners. Each check has a positive control that
 * sees traffic when the proxy or the init script is left out.
 * Returns { browser, page }; close the browser when done.
 */
export async function launchOffline() {
  const { chromium } = createRequire(import.meta.url)("playwright");
  const browser = await chromium.launch(offlineLaunch());
  const context = await browser.newContext();
  await context.addInitScript(removeWebRtc);
  const ORIGIN = "https://hostile.surimono.test/";
  await context.route("**/*", (route) =>
    route.request().url() === ORIGIN ? route.fulfill({ contentType: "text/html", body: "<!doctype html><title>hostile</title>" }) : route.abort(),
  );
  await context.routeWebSocket(/.*/, (ws) => ws.close({ code: 1008, reason: "no network in the hostile suite" }));
  const page = await context.newPage();
  await page.goto(ORIGIN);
  return { browser, page };
}
