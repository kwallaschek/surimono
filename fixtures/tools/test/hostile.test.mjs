// The hostile manifest reader (lib/hostile.mjs) against Python's tomllib, and
// on edge cases:  node --test fixtures/tools/test/*.test.mjs

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

test("launchOffline: a Worker's WebSocket and fetch reach nothing, not even loopback", async (t) => {
  let launchOffline;
  try {
    ({ launchOffline } = await import("../lib/hostile.mjs"));
    const { createRequire } = await import("node:module");
    const require = createRequire(import.meta.url);
    const fs = await import("node:fs");
    if (!fs.existsSync(require("playwright").chromium.executablePath())) return t.skip("Playwright's Chromium is not installed");
  } catch (e) {
    return t.skip(`playwright not available: ${e.message}`);
  }
  const net = await import("node:net");
  const connections = [];
  const server = net.createServer((socket) => {
    connections.push(socket.remoteAddress);
    socket.destroy();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const { browser, page } = await launchOffline();
  try {
    const result = await page.evaluate(async (port) => {
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
    assert.doesNotMatch(result, /fetched|open after/, result);
  } finally {
    await browser.close();
    await new Promise((resolve) => server.close(resolve));
  }
  assert.deepEqual(connections, [], "no TCP connection reached the listener");
});
