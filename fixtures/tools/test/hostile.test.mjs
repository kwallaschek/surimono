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

/**
 * In `page`, its about:blank iframe and a window.open window: an
 * RTCPeerConnection with a STUN server at 127.0.0.1:port, gathering
 * candidates for up to 3 s. Also reports whether a dedicated Worker has
 * RTCPeerConnection. Returns one line per place.
 */
function webRtcProbe(page, port) {
  return page.evaluate(async (port) => {
    const out = [];
    const attempt = async (w, where) => {
      const PC = w.RTCPeerConnection || w.webkitRTCPeerConnection;
      if (!PC) return out.push(`${where}: no RTCPeerConnection`);
      try {
        const pc = new PC({ iceServers: [{ urls: `stun:127.0.0.1:${port}` }] });
        pc.createDataChannel("probe");
        await pc.setLocalDescription(await pc.createOffer());
        await new Promise((res) => {
          pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && res();
          setTimeout(res, 3000);
        });
        out.push(`${where}: gathered`);
      } catch (e) {
        out.push(`${where}: ${e.name}`);
      }
    };
    await attempt(window, "page");
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    await attempt(frame.contentWindow, "iframe");
    const popup = window.open("about:blank");
    if (popup) await attempt(popup, "window.open");
    const worker = new Worker(URL.createObjectURL(new Blob(["postMessage(typeof RTCPeerConnection)"], { type: "text/javascript" })));
    out.push(`worker: ${await new Promise((res) => (worker.onmessage = (e) => res(e.data)))} RTCPeerConnection`);
    return out.join("; ");
  }, port);
}

test("launchOffline: the page has no WebRTC, so a STUN request from the page or its frames sends nothing", async (t) => {
  const { chromium, skip } = await playwrightChromium();
  if (skip) return t.skip(skip);
  const { OFFLINE_LAUNCH, launchOffline } = await import("../lib/hostile.mjs");
  const dgram = await import("node:dgram");
  const socket = dgram.createSocket("udp4");
  let datagrams = 0;
  socket.on("message", () => datagrams++);
  await new Promise((resolve) => socket.bind(0, "127.0.0.1", resolve));
  const port = socket.address().port;
  const settle = () => new Promise((resolve) => setTimeout(resolve, 500));
  try {
    // Positive control: the same proxy without the init script. STUN goes
    // over UDP past the proxy and reaches the socket.
    const proxied = await chromium.launch(OFFLINE_LAUNCH);
    try {
      const result = await webRtcProbe(await proxied.newPage(), port);
      assert.match(result, /page: gathered/, result);
    } finally {
      await proxied.close();
    }
    await settle();
    assert.ok(datagrams >= 1, `the control's STUN requests reached the socket (${datagrams} datagrams)`);
    datagrams = 0;

    const { browser, page } = await launchOffline();
    try {
      const result = await webRtcProbe(page, port);
      assert.equal(result, "page: no RTCPeerConnection; iframe: no RTCPeerConnection; window.open: no RTCPeerConnection; worker: undefined RTCPeerConnection");
      await settle();
    } finally {
      await browser.close();
    }
    assert.equal(datagrams, 0, "no UDP datagram reached the socket");
  } finally {
    socket.close();
  }
});
