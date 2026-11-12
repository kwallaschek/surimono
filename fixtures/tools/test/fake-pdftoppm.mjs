#!/usr/bin/env node
// A stand-in for poppler's pdftoppm, so the printer tooling's tests need no
// poppler: they test the tools, not a rasteriser (GitHub's runners have none
// until a later step installs it). It answers what lib/fixtures.mjs asks:
//
//   pdftoppm -v                          the version, on stderr as poppler prints it
//   pdftoppm -r DPI -png IN.pdf PREFIX   one PNG, PREFIX-1.png
//
// and only understands test/fake-fulgur.mjs's one-page PDF: the page is its
// MediaBox at DPI (rounded up, as poppler does), white, with every `x y w h re`
// rectangle of the content stream in black. Never a source of results.

import fs from "node:fs";
import { PNG } from "pngjs";

const args = process.argv.slice(2);
if (args[0] === "-v") {
  console.error("pdftoppm version 0.0.0 (fake-pdftoppm.mjs)");
  process.exit(0);
}
const dpi = Number(args[args.indexOf("-r") + 1]);
const [input, prefix] = args.slice(-2);
if (!args.includes("-png") || !(dpi > 0) || !input || !prefix || !fs.existsSync(input)) {
  console.error("usage: fake-pdftoppm.mjs -r DPI -png IN.pdf PREFIX");
  process.exit(99);
}
const pdf = fs.readFileSync(input, "latin1");
const box = /\/MediaBox \[0 0 ([\d.]+) ([\d.]+)\]/.exec(pdf);
if (!box) {
  console.error(`fake-pdftoppm.mjs: ${input} isn't fake-fulgur.mjs's PDF`);
  process.exit(1);
}
const scale = dpi / 72;
const width = Math.ceil(Number(box[1]) * scale);
const height = Math.ceil(Number(box[2]) * scale);
const png = new PNG({ width, height });
png.data.fill(255);
for (const m of pdf.matchAll(/([\d.]+) ([\d.]+) ([\d.]+) ([\d.]+) re\b/g)) {
  const [x, y, w, h] = m.slice(1).map((v) => Number(v) * scale);
  // PDF's origin is the bottom left corner, a PNG's the top left.
  for (let row = Math.round(height - y - h); row < Math.round(height - y); row++) {
    for (let col = Math.round(x); col < Math.round(x + w); col++) {
      if (row < 0 || row >= height || col < 0 || col >= width) continue;
      const i = (row * width + col) * 4;
      png.data[i] = png.data[i + 1] = png.data[i + 2] = 0;
    }
  }
}
fs.writeFileSync(`${prefix}-1.png`, PNG.sync.write(png));
