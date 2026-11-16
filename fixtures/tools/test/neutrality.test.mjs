// The neutrality cases' format and neutrality.mjs's verdicts, with no Chromium.
//   node --test fixtures/tools/test/*.test.mjs

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { PNG } from "pngjs";
import { judgeLoad, judgePage, judgePrinterSafe, listCases, metaOf, printerSafeChange, resourcesOf, samePixels } from "../neutrality.mjs";

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
    // printer_safe, and loads with a page_charset for each, parse.
    metaOf(c.dir);
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

function withMeta(text, f) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "neutrality-test-"));
  try {
    fs.writeFileSync(path.join(tmp, "meta.toml"), text);
    return f(tmp);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

test("meta.toml says what the printer-safe pass does, and how and with what charset each load is read", () => {
  const ok = withMeta('printer_safe = "media"\n', metaOf);
  assert.deepEqual(ok, { printerSafe: "media", loads: ["http"], pageCharset: {}, mojibake: [] });
  const loads = withMeta('printer_safe = "dom"\nloads = ["http", "file", "http-no-charset"]\npage_charset.http = "UTF-8"\npage_charset.file = "UTF-8"\npage_charset.http-no-charset = "windows-1252"\nmojibake = ["http-no-charset"]\n', metaOf);
  assert.deepEqual(loads.pageCharset, { http: "UTF-8", file: "UTF-8", "http-no-charset": "windows-1252" });
  assert.deepEqual(loads.mojibake, ["http-no-charset"]);
  assert.throws(() => withMeta('title = "x"\n', metaOf), /printer_safe must be one of dom, sheet, media/);
  assert.throws(() => withMeta('printer_safe = "left"\n', metaOf), /printer_safe must be one of/);
  assert.throws(() => withMeta('printer_safe = "dom"\nloads = ["ftp"]\n', metaOf), /loads must be distinct names/);
  assert.throws(() => withMeta('printer_safe = "dom"\nloads = ["http", "file"]\npage_charset.http = "UTF-8"\n', metaOf), /no page_charset\.file/);
  assert.throws(() => withMeta('printer_safe = "dom"\npage_charset.file = "UTF-8"\n', metaOf), /names a load the case doesn't list/);
  // A load can be mojibake only where the page isn't decoded as UTF-8: never a way to skip a comparison.
  assert.throws(() => withMeta('printer_safe = "dom"\nloads = ["http", "file"]\npage_charset.http = "UTF-8"\npage_charset.file = "UTF-8"\nmojibake = ["file"]\n', metaOf), /needs a page_charset other than UTF-8/);
  assert.throws(() => withMeta('printer_safe = "dom"\nmojibake = ["file"]\n', metaOf), /isn't one of its loads/);
});

test("the printer-safe change is dom, only added sheets, or more", () => {
  const plain = '<html><head><title>t</title><style>@media print{p{color:red}}</style></head><body><p>x</p></body></html>';
  assert.equal(printerSafeChange(plain, plain), "dom");
  const sheet = plain.replace("</title>", "</title><style>@layer{svg{width:1px}}</style>");
  assert.equal(printerSafeChange(plain, sheet), "sheet");
  assert.equal(printerSafeChange(plain, sheet.replace("@media print", "@media all")), "media");
  assert.equal(printerSafeChange(plain, plain.replace("@media print{p{color:red}}", "/**/")), "media", "a silenced <style> is more than a sheet");
  // A second copy of a <style> the page already has is an added sheet.
  assert.equal(printerSafeChange(plain, plain.replace("</title>", "</title><style>@media print{p{color:red}}</style>")), "sheet");
});

test("a load compares the output with the page unless the case says the page is mojibake there", () => {
  const meta = { pageCharset: { http: "UTF-8", "http-no-charset": "windows-1252" }, mojibake: ["http-no-charset"] };
  const same = { page_charset: "UTF-8", output_charset: "UTF-8", pixels_same: true, text_same: true };
  assert.deepEqual(judgeLoad("http", meta, same), { ok: true, result: "same" });
  assert.deepEqual(judgeLoad("http", meta, { ...same, output_charset: "windows-1252", pixels_same: false }), { ok: false, result: "differs: decodes as windows-1252, pixels" });
  assert.deepEqual(judgeLoad("http", meta, { ...same, text_same: false }), { ok: false, result: "differs: text" });
  assert.equal(judgeLoad("http", meta, { ...same, page_charset: "windows-1252" }).ok, false, "the page's own decoding is held to meta.toml");
  const mojibake = { page_charset: "windows-1252", output_charset: "GBK", pixels_same: false, text_same: false };
  assert.deepEqual(judgeLoad("http-no-charset", meta, mojibake), { ok: true, result: "not compared" });
  assert.equal(judgeLoad("http-no-charset", meta, { ...mojibake, page_charset: "UTF-8" }).ok, false, "a page that decodes as UTF-8 there is compared again");
  // A load the case doesn't list as mojibake is compared, whatever the page decodes as.
  assert.deepEqual(judgeLoad("file", { pageCharset: { file: "replacement" }, mojibake: [] }, { ...mojibake, page_charset: "replacement" }), { ok: false, result: "differs: decodes as GBK, pixels, text" });
  assert.deepEqual(judgeLoad("http", meta, { page_charset: "UTF-8" }), { ok: true, result: "page only" });
});

test("a printer_safe that moved from left alone to rewritten fails the case, naming both and what to do", () => {
  assert.deepEqual(judgePrinterSafe("c-alt", "media", "media"), { ok: true });
  const v = judgePrinterSafe("c-alt", "dom", "media");
  assert.equal(v.ok, false);
  assert.match(v.message, /^c-alt: meta\.toml says printer_safe = "dom".*this run saw "media"/);
  assert.match(v.message, /set printer_safe = "media" in fixtures\/neutrality\/c-alt\/meta\.toml in the same pull request/);
});

test("a printer_safe that moved from rewritten to left alone fails the case too", () => {
  const v = judgePrinterSafe("g-nest-child", "media", "dom");
  assert.equal(v.ok, false);
  assert.match(v.message, /meta\.toml says printer_safe = "media".*this run saw "dom"/);
  assert.equal(judgePrinterSafe("s-svg-style", "sheet", "media").ok, false, "sheet to media as well");
});
