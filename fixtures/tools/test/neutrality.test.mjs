// The neutrality cases' format and neutrality.mjs's verdicts, with no Chromium.
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { PNG } from "pngjs";
import { judgePage, listCases, resourcesOf, samePixels } from "../neutrality.mjs";

const toml = (dir) => fs.readFileSync(path.join(dir, "meta.toml"), "utf8");

test("every case says what it tests and where it comes from, and names only files it has", () => {
  const cases = listCases();
  assert.ok(cases.length >= 30, `${cases.length} cases`);
  for (const c of cases) {
    const meta = toml(c.dir);
    for (const key of ["title", "what", "from", "changed", "links", "resources"]) assert.match(meta, new RegExp(`^${key} = `, "m"), `${c.name}: no ${key}`);
    const links = [.../^links = \[([^\]]*)\]$/m.exec(meta)[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    const input = fs.readFileSync(path.join(c.dir, "input.html"), "utf8");
    for (const f of links) {
      assert.ok(fs.existsSync(path.join(c.dir, f)), `${c.name}: links ${f}, which isn't there`);
      assert.ok(input.includes(f), `${c.name}: links ${f}, which the page doesn't name`);
    }
    for (const r of resourcesOf(c.dir)) assert.ok(links.includes(r), `${c.name}: resource ${r} isn't one of its links`);
    // The print may need no system font: every family named is a fixture font.
    for (const m of input.matchAll(/font(?:-family)?\s*:[^;}"]*?(\b(?:serif|sans-serif|monospace|cursive|fantasy|system-ui)\b)/g)) assert.fail(`${c.name}: names the generic family ${m[1]}`);
    assert.match(input, /@font-face\{font-family:"Noto Sans";src:url\("\.\.\/\.\.\/fonts\/NotoSans-Regular\.ttf"\)\}/, `${c.name}: no fixture font`);
  }
});

test("listCases refuses an unknown case and resourcesOf a meta.toml without the line", () => {
  assert.throws(() => listCases(["no-such-case"]), /no such neutrality case: no-such-case/);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "neutrality-test-"));
  try {
    fs.writeFileSync(path.join(tmp, "meta.toml"), 'title = "x"\n');
    assert.throws(() => resourcesOf(tmp), /no resources = \[\.\.\.\] line/);
    fs.writeFileSync(path.join(tmp, "meta.toml"), 'resources = ["a.css", "b.css"]\n');
    assert.deepEqual(resourcesOf(tmp), ["a.css", "b.css"]);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

function image(width, height, paint = () => {}) {
  const png = new PNG({ width, height });
  png.data.fill(255);
  paint(png);
  return png;
}
const dot = (x, y, v = 0) => (png) => {
  const i = (y * png.width + x) * 4;
  png.data[i] = png.data[i + 1] = png.data[i + 2] = v;
};

test("a page is identical, reproduced or changed as reference.mjs judges, and an output must be the same pixels", () => {
  const blank = image(40, 40);
  assert.equal(judgePage(blank, image(40, 40)).result, "identical");
  // A solid block moved: not anti-aliasing.
  const block = (x0) => (png) => {
    for (let y = 10; y < 20; y++) for (let x = x0; x < x0 + 10; x++) dot(x, y)(png);
  };
  assert.equal(judgePage(image(40, 40, block(5)), image(40, 40, block(20))).result, "changed");
  assert.equal(judgePage(blank, image(40, 41)).result, "changed");
  assert.equal(samePixels([blank], [image(40, 40)]), true);
  assert.equal(samePixels([blank], [image(40, 40, dot(3, 3, 254))]), false, "one pixel off by one is a difference");
  assert.equal(samePixels([blank], [blank, blank]), false);
});
