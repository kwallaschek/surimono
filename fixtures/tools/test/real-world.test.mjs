// The real-world pages stay offline and deterministic, with no Chrome:
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { FIXTURES_DIR, KINDS, listAllFixtures, listFixtures } from "../lib/fixtures.mjs";

const pages = listFixtures("real-world");

test("each library has a page, and noscript-message has no library", () => {
  assert.deepEqual(pages.map((f) => f.name), ["alpine-invoice", "d3-charts", "highlightjs-code", "katex-autorender", "marked-report", "noscript-message"]);
});

test("every kind is listed, and a name is found in any kind", () => {
  assert.deepEqual(KINDS, ["islands", "real-world"]);
  assert.deepEqual(listAllFixtures(["d3-charts", "echarts-pie-ja"]).map((f) => f.kind).sort(), ["islands", "real-world"]);
  assert.throws(() => listAllFixtures(["no-such-page"]), /no such fixture/);
});

for (const fixture of pages) {
  const html = fs.readFileSync(path.join(fixture.dir, "input.html"), "utf8");

  test(`${fixture.name}: everything it loads is committed under fixtures/`, () => {
    const refs = [...html.matchAll(/<(?:script|link)\b[^>]*\b(?:src|href)="([^"]+)"/g), ...html.matchAll(/url\("([^"]+)"\)/g)].map((m) => m[1]);
    assert.ok(refs.length > 0);
    for (const ref of refs) {
      assert.doesNotMatch(ref, /^[a-z]+:|^\/\//i, ref);
      const file = path.resolve(fixture.dir, ref);
      assert.ok(file.startsWith(FIXTURES_DIR + path.sep) && fs.existsSync(file), ref);
    }
  });

  test(`${fixture.name}: each font a stylesheet names has a vendored first choice`, () => {
    for (const [, href] of html.matchAll(/<link\b[^>]*\bhref="([^"]+\.css)"/g)) {
      const sheet = path.resolve(fixture.dir, href);
      for (const [, src] of fs.readFileSync(sheet, "utf8").matchAll(/src:\s*([^;}]+)/g)) {
        const first = /url\(["']?([^"')]+)/.exec(src)[1];
        assert.ok(fs.existsSync(path.resolve(path.dirname(sheet), first)), first);
      }
    }
  });

  test(`${fixture.name}: no clock or random source, no measuring`, () => {
    assert.doesNotMatch(html, /Math\.random|Date\.now|new Date|performance\.now|getBBox|getComputedTextLength|getBoundingClientRect/);
  });

  test(`${fixture.name}: has its references`, () => {
    for (const f of ["chrome-1.png", "dom.html", "reference.toml"]) assert.ok(fs.existsSync(path.join(fixture.dir, "reference", f)), f);
  });
}
