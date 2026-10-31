// The hostile manifest reader (lib/hostile.mjs) against Python's tomllib and
// on edge cases, and launchOffline's network isolation, each network check
// with a positive control:  node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { HOSTILE_DIR, hostileCases } from "../lib/hostile.mjs";

test("hostileCases reads manifest.toml exactly as tomllib does", () => {
  const py = `
import json, sys, tomllib
m = tomllib.load(open(sys.argv[1], "rb"))
out = []
for c in m["case"]:
    if "steps" in c:
        out += [{"id": c["id"], "step": i, "file": s["file"]} for i, s in enumerate(c["steps"], 1)]
    else:
        out.append({"id": c["id"], "step": None, "file": c["file"]})
print(json.dumps(out))`;
  const r = spawnSync("python3", ["-I", "-c", py, path.join(HOSTILE_DIR, "manifest.toml")], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(hostileCases(), JSON.parse(r.stdout));
});

test("hostileCases: steps become numbered entries; a case without file or steps is an error", () => {
  const text = `[limits]\nmax_wall_ms = 1\n\n[[case]]\nid = "a"\nfile = "a.js"\nexpect = "Ok"\n\n[[case]]\nid = "b"\nsteps = [{ file = "b1.js", expect = "Ok" }, { file = "b2.js", expect = "Ok" }]\nabout = "x"\n`;
  assert.deepEqual(hostileCases(text), [
    { id: "a", step: null, file: "a.js" },
    { id: "b", step: 1, file: "b1.js" },
    { id: "b", step: 2, file: "b2.js" },
  ]);
  assert.throws(() => hostileCases(`[[case]]\nid = "c"\nexpect = "Ok"\n`), /neither file nor steps/);
});

/** Playwright's chromium, or a reason to skip the browser tests. */
async function playwrightChromium() {
  try {
    const { createRequire } = await import("node:module");
    const require = createRequire(import.meta.url);
    const fs = await import("node:fs");
    const { chromium } = require("playwright");
    if (!fs.existsSync(chromium.executablePath())) return { skip: "Playwright's Chromium is not installed" };
    return { chromium };
  } catch (e) {
    return { skip: `playwright not available: ${e.message}` };
  }
}

/**
 * From a blob-URL dedicated Worker of `page` (as hostile-chrome.mjs runs
 * cases), a WebSocket and a fetch to 127.0.0.1:port; returns what happened.
 */
function workerProbe(page, port) {
  return page.evaluate(async (port) => {
    const src = `(async () => {
      const ws = await new Promise((res) => {
        try {
          const w = new WebSocket("ws://127.0.0.1:${port}/worker");
          w.onclose = (e) => res("closed " + e.code);
          w.onerror = () => {};
          setTimeout(() => res("open after 3 s: " + w.readyState), 3000);
        } catch (e) { res("threw " + e.name); }
      });
      let f;
      try { await fetch("http://127.0.0.1:${port}/worker"); f = "fetched"; } catch (e) { f = "fetch " + e.name; }
      postMessage(ws + "; " + f);
    })();`;
    const w = new Worker(URL.createObjectURL(new Blob([src], { type: "text/javascript" })));
    return await new Promise((res) => {
      w.onmessage = (e) => res(e.data);
      setTimeout(() => res("worker timeout"), 8000);
    });
  }, port);
}

/** A local TCP listener that records every connection it accepts. */
async function tcpListener() {
  const net = await import("node:net");
  const connections = [];
  const server = net.createServer((socket) => {
    connections.push(socket.remoteAddress);
    socket.destroy();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { connections, port: server.address().port, close: () => new Promise((resolve) => server.close(resolve)) };
}

test("launchOffline: a Worker's WebSocket and fetch reach nothing, not even loopback", async (t) => {
  const { chromium, skip } = await playwrightChromium();
  if (skip) return t.skip(skip);
  const { launchOffline } = await import("../lib/hostile.mjs");
  const listener = await tcpListener();
  try {
    // Positive control: the same probe without the proxy and the routes
    // reaches the listener, so the probe and the listener work.
    const plain = await chromium.launch({ channel: "chromium" });
    try {
      const result = await workerProbe(await plain.newPage(), listener.port);
      assert.doesNotMatch(result, /worker timeout/, result);
    } finally {
      await plain.close();
    }
    assert.ok(listener.connections.length >= 1, `the control reached the listener (${listener.connections.length} connections)`);
    listener.connections.length = 0;

    const { browser, page } = await launchOffline();
    try {
      const result = await workerProbe(page, listener.port);
      assert.doesNotMatch(result, /fetched|open after|worker timeout/, result);
    } finally {
      await browser.close();
    }
  } finally {
    await listener.close();
  }
  assert.deepEqual(listener.connections, [], "no TCP connection reached the listener");
});

/** The places webRtcProbe tries, in order. */
const PLACES = ["page", "iframe", "sandboxed srcdoc", "window.open"];

/**
 * An RTCPeerConnection with a STUN server at 127.0.0.1:ports[place], gathering
 * candidates for up to 3 s, in each of PLACES: `page`, an about:blank iframe,
 * an `<iframe sandbox="allow-scripts" srcdoc>` (an opaque origin, which
 * answers by postMessage) and a window.open window. Also reports whether a
 * dedicated Worker has RTCPeerConnection. Returns one line per place.
 */
function webRtcProbe(page, ports) {
  return page.evaluate(async ([places, ports]) => {
    // Self-contained: the sandboxed frame runs it from its source text.
    async function tryStun(w, port) {
      const PC = w.RTCPeerConnection || w.webkitRTCPeerConnection;
      if (!PC) return "no RTCPeerConnection";
      try {
        const pc = new PC({ iceServers: [{ urls: `stun:127.0.0.1:${port}` }] });
        pc.createDataChannel("probe");
        await pc.setLocalDescription(await pc.createOffer());
        await new Promise((res) => {
          pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && res();
          setTimeout(res, 3000);
        });
        return "gathered";
      } catch (e) {
        return e.name;
      }
    }
    const out = {};
    out.page = await tryStun(window, ports.page);
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    out.iframe = await tryStun(frame.contentWindow, ports.iframe);
    out["sandboxed srcdoc"] = await new Promise((res) => {
      const onMessage = (e) => {
        if (e.data && e.data.probe === "sandboxed") {
          removeEventListener("message", onMessage);
          res(e.data.result);
        }
      };
      addEventListener("message", onMessage);
      setTimeout(() => res("no answer"), 8000);
      const sandboxed = document.createElement("iframe");
      sandboxed.sandbox = "allow-scripts";
      sandboxed.srcdoc = `<script>(${tryStun})(window, ${ports["sandboxed srcdoc"]}).then((result) => parent.postMessage({ probe: "sandboxed", result }, "*"));<\/script>`;
      document.body.appendChild(sandboxed);
    });
    const popup = window.open("about:blank");
    out["window.open"] = popup ? await tryStun(popup, ports["window.open"]) : "blocked";
    const worker = new Worker(URL.createObjectURL(new Blob(["postMessage(typeof RTCPeerConnection)"], { type: "text/javascript" })));
    const inWorker = await new Promise((res) => (worker.onmessage = (e) => res(e.data)));
    return [...places.map((p) => `${p}: ${out[p]}`), `worker: ${inWorker} RTCPeerConnection`].join("; ");
  }, [PLACES, ports]);
}

/** One local UDP socket per place, each counting the datagrams it receives. */
async function udpSockets(places) {
  const dgram = await import("node:dgram");
  const sockets = {};
  for (const place of places) {
    const socket = dgram.createSocket("udp4");
    const entry = { socket, datagrams: 0 };
    socket.on("message", () => entry.datagrams++);
    await new Promise((resolve) => socket.bind(0, "127.0.0.1", resolve));
    sockets[place] = entry;
  }
  return {
    ports: Object.fromEntries(places.map((p) => [p, sockets[p].socket.address().port])),
    counts: () => Object.fromEntries(places.map((p) => [p, sockets[p].datagrams])),
    reset: () => places.forEach((p) => (sockets[p].datagrams = 0)),
    close: () => places.forEach((p) => sockets[p].socket.close()),
  };
}

test("launchOffline: no frame of the page has WebRTC, so a STUN request from any of them sends nothing", async (t) => {
  const { chromium, skip } = await playwrightChromium();
  if (skip) return t.skip(skip);
  const { offlineLaunch, launchOffline } = await import("../lib/hostile.mjs");
  const udp = await udpSockets(PLACES);
  const settle = () => new Promise((resolve) => setTimeout(resolve, 500));
  try {
    // Positive control: the same launch options without the init script.
    // STUN goes over UDP past the proxy, and every place's socket hears it.
    const proxied = await chromium.launch(offlineLaunch());
    try {
      const result = await webRtcProbe(await proxied.newPage(), udp.ports);
      for (const place of PLACES) assert.match(result, new RegExp(`${place}: gathered`), result);
    } finally {
      await proxied.close();
    }
    await settle();
    for (const [place, n] of Object.entries(udp.counts())) assert.ok(n >= 1, `the control's STUN requests from ${place} reached its socket (${n} datagrams)`);
    udp.reset();

    const { browser, page } = await launchOffline();
    try {
      const result = await webRtcProbe(page, udp.ports);
      assert.equal(result, `${PLACES.map((p) => `${p}: no RTCPeerConnection`).join("; ")}; worker: undefined RTCPeerConnection`);
      await settle();
    } finally {
      await browser.close();
    }
    assert.deepEqual(udp.counts(), Object.fromEntries(PLACES.map((p) => [p, 0])), "no UDP datagram reached any socket");
  } finally {
    udp.close();
  }
});

test("offlineLaunch keeps Playwright's own disabled features", async (t) => {
  // Chromium keeps only the last --disable-features switch. HttpsUpgrades is
  // one Playwright disables: with it back on, http:// navigations become https://.
  const { chromium, skip } = await playwrightChromium();
  if (skip) return t.skip(skip);
  const { offlineLaunch } = await import("../lib/hostile.mjs");
  const firstUrl = async (options) => {
    const browser = await chromium.launch(options);
    try {
      const context = await browser.newContext();
      const urls = [];
      await context.route("**/*", (route) => {
        urls.push(route.request().url());
        return route.fulfill({ contentType: "text/html", body: "<p>ok" });
      });
      await (await context.newPage()).goto("http://www.example.com/upgrade");
      return urls[0];
    } finally {
      await browser.close();
    }
  };
  // Positive control: a lone --disable-features replaces Playwright's list.
  assert.equal(await firstUrl({ channel: "chromium", args: ["--disable-features=IsolateSandboxedIframes"] }), "https://www.example.com/upgrade");
  assert.equal(await firstUrl(offlineLaunch()), "http://www.example.com/upgrade");
  assert.match(offlineLaunch().args.join(" "), /--disable-features=\S*\bHttpsUpgrades\b\S*\bIsolateSandboxedIframes\b/);
});

test("offlineLaunch's --disable-features is Playwright's real one plus IsolateSandboxedIframes", async (t) => {
  // Review of #50: offlineLaunch reads Playwright's list from its bundled
  // source with a regex, which could silently drop a name. The outside truth
  // is the command line Chromium itself reports (chrome://version).
  const { chromium, skip } = await playwrightChromium();
  if (skip) return t.skip(skip);
  const { offlineLaunch } = await import("../lib/hostile.mjs");
  const disabledOf = async (options) => {
    const browser = await chromium.launch(options);
    try {
      const page = await browser.newPage();
      await page.goto("chrome://version");
      const line = await page.locator("#command_line").innerText();
      const all = [...line.matchAll(/--disable-features=(\S*)/g)];
      assert.ok(all.length >= 1, `no --disable-features in ${line.slice(0, 200)}`);
      return all[all.length - 1][1].split(",").filter(Boolean); // Chromium keeps the last one
    } finally {
      await browser.close();
    }
  };
  const playwrights = await disabledOf({ channel: "chromium" });
  assert.ok(playwrights.includes("HttpsUpgrades"), playwrights.join(","));
  const ours = offlineLaunch().args.find((a) => a.startsWith("--disable-features=")).slice("--disable-features=".length).split(",");
  assert.deepEqual(ours, [...playwrights, "IsolateSandboxedIframes"]);
  // And that is what Chromium runs with.
  assert.deepEqual(await disabledOf(offlineLaunch()), ours);
});
