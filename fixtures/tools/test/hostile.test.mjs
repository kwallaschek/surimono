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
