#!/usr/bin/env node
// A stand-in for the fulgur CLI, to test the printer tooling (printers.mjs and the
// --fulgur, --measure options) without a printer. It answers `--version` and
// `render [--font F]... --creation-date D -o <out.pdf> <in.html>` with a one-page
// PDF that has nothing to do with the HTML: a black square on a page.
//
//   FAKE_FULGUR_VERSION   what --version says after "fulgur " (default 0.0.0-fake)
//   FAKE_FULGUR_PAGE      "a4" (default) or "letter": the page size
//   FAKE_FULGUR_FAIL      "exit": exit 1 with a message; "abort": die by SIGABRT;
//                         "hang": never finish
// Never a source of results.

import fs from "node:fs";

const args = process.argv.slice(2);
if (args[0] === "--version") {
  console.log(`fulgur ${process.env.FAKE_FULGUR_VERSION || "0.0.0-fake"}`);
  process.exit(0);
}
const out = args[args.indexOf("-o") + 1];
const input = args[args.length - 1];
if (args[0] !== "render" || !args.includes("-o") || !out || !fs.existsSync(input)) {
  console.error("usage: fake-fulgur.mjs render [--font F]... -o <out.pdf> <in.html>");
  process.exit(2);
}
if (process.env.FAKE_FULGUR_FAIL === "exit") {
  console.error("error: fake-fulgur was told to fail");
  process.exit(1);
}
if (process.env.FAKE_FULGUR_FAIL === "abort") process.kill(process.pid, "SIGABRT");
if (process.env.FAKE_FULGUR_FAIL === "hang") Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0);

// 210 x 297 mm and 8.5 x 11 in, in points.
const [w, h] = process.env.FAKE_FULGUR_PAGE === "letter" ? [612, 792] : [595.276, 841.89];
const stream = "0 0 0 rg 100 600 100 100 re f\n";
const objects = [
  "<< /Type /Catalog /Pages 2 0 R >>",
  "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
  `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${w} ${h}] /Contents 4 0 R /Resources << >> >>`,
  `<< /Length ${stream.length} >>\nstream\n${stream}endstream`,
];
let pdf = "%PDF-1.4\n";
const offsets = [];
objects.forEach((body, i) => {
  offsets.push(pdf.length);
  pdf += `${i + 1} 0 obj\n${body}\nendobj\n`;
});
const xref = pdf.length;
pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, "0")} 00000 n \n`).join("")}`;
pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
fs.writeFileSync(out, pdf, "latin1");
