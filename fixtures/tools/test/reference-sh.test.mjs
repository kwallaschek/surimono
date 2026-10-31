// reference.sh's gfx Chrome checks, with stand-ins for node, cargo, npm, npx and
// pdftoppm on PATH, so neither Chromium nor a build runs:
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const script = path.resolve(here, "..", "reference.sh");

// node: the reference steps succeed; the gfx scripts log that they ran and exit
// with $STUB_INLINE_EXIT / $STUB_OUTLINE_EXIT; `node -e` (reading cargo metadata) prints a target dir.
const STUBS = {
  node: `#!/bin/bash
case "$1" in
  -e) cat >/dev/null; echo /nonexistent-target ;;
  */svg_inline.mjs) echo svg_inline >> "$STUB_LOG"; exit "\${STUB_INLINE_EXIT:-0}" ;;
  */svg_outline.mjs) echo svg_outline >> "$STUB_LOG"; exit "\${STUB_OUTLINE_EXIT:-0}" ;;
esac
exit 0
`,
  cargo: `#!/bin/bash
[ "$1" = metadata ] && echo '{}'
exit 0
`,
  npm: "#!/bin/bash\nexit 0\n",
  npx: "#!/bin/bash\nexit 0\n",
  pdftoppm: "#!/bin/bash\nexit 0\n",
};

const made = [];
test.after(() => {
  for (const d of made) fs.rmSync(d, { recursive: true, force: true });
});

/** Runs reference.sh with the stand-ins in `tools` on PATH (and nothing else of ours). */
function run({ tools = Object.keys(STUBS), env = {} } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "reference-sh-test-"));
  made.push(dir);
  const bin = path.join(dir, "bin");
  fs.mkdirSync(bin);
  for (const name of tools) {
    fs.writeFileSync(path.join(bin, name), STUBS[name], { mode: 0o755 });
  }
  const log = path.join(dir, "ran.log");
  const r = spawnSync("/bin/bash", [script, path.join(dir, "out")], {
    encoding: "utf8",
    env: { PATH: `${bin}:/usr/bin:/bin`, HOME: dir, STUB_LOG: log, ...env },
  });
  const ran = fs.existsSync(log) ? fs.readFileSync(log, "utf8").trim().split("\n") : [];
  return { status: r.status, stderr: r.stderr, stdout: r.stdout, ran };
}

test("both gfx checks run and pass", () => {
  const r = run();
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.ran, ["svg_inline", "svg_outline"]);
});

test("svg_inline failing still runs svg_outline, and fails the run", () => {
  const r = run({ env: { STUB_INLINE_EXIT: "1" } });
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.deepEqual(r.ran, ["svg_inline", "svg_outline"]);
});

test("svg_outline failing fails the run", () => {
  const r = run({ env: { STUB_OUTLINE_EXIT: "1" } });
  assert.equal(r.status, 1, r.stdout + r.stderr);
});

test("without cargo the checks are skipped locally", () => {
  const r = run({ tools: ["node", "npm", "npx", "pdftoppm"] });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stderr, /skipped, cargo not found/);
  assert.deepEqual(r.ran, []);
});

test("without cargo on a CI runner the run fails", () => {
  const r = run({ tools: ["node", "npm", "npx", "pdftoppm"], env: { CI: "true" } });
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stderr, /cargo not found on a CI runner/);
});
