#!/usr/bin/env node
// Generates the two long documents of the benchmark's page set (bench/README.md,
// "The page set"), next to the six real-world fixture pages:
//
//   long-report.html  an annual report: a contents list, 10 chapters of 10
//                     sections of 50 paragraphs (5,000 paragraphs) and 6 tables
//                     of figures. Its script numbers the chapters, sections and
//                     tables, builds the contents list and fills in each table's
//                     totals row.
//   long-table.html   a general ledger: one table of 2,000 entries with short
//                     cells. Its script formats the amounts and fills in the
//                     running balance and the totals row.
//
//   node bench/pages/gen-pages.mjs [--out DIR]   # write both (default bench/.cache/pages)
//   node bench/pages/gen-pages.mjs --check       # exit 1 unless the bytes match bench/pages/SHA256SUMS
//   node bench/pages/gen-pages.mjs --sums        # rewrite bench/pages/SHA256SUMS (after a deliberate change)
//
// The text comes from a seeded PRNG over a fixed word list: no clock, no
// locale, no Math.random, so every machine and Node version writes the same
// bytes. The pages are meant to look like documents people print, with an
// ordinary stylesheet; nothing in them is shaped to stress a printer.
//
// Fonts are the fixture fonts through @font-face URLs relative to
// bench/.cache/pages, where bench.py has them written, so Chrome draws with the
// same files the browserless printers get. The URLs are fixed, so the bytes are
// the same in any --out folder; only there do the fonts resolve.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const SUMS = path.join(HERE, "SHA256SUMS");
export const DEFAULT_OUT = path.join(ROOT, "bench", ".cache", "pages");
const FONTS_URL = "../../../fixtures/fonts";

/** mulberry32: a small, well-known 32-bit PRNG. Same seed, same sequence. */
function prng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = (
  "the a of and to in for on with by from at as that this these each every our its their " +
  "revenue orders margin costs growth market customers service delivery quarter year region team " +
  "product products invoice invoices accounts payments receivables payables cash flow budget forecast " +
  "plan review results report reports figures data system systems process processes supplier suppliers " +
  "contract contracts staff office offices branch branches sales volume price prices demand supply " +
  "operating expenses income tax capital investment project projects schedule target targets risk risks " +
  "quality audit control controls policy standard standards customer client clients partner partners " +
  "increased decreased remained improved reduced expanded completed started opened closed moved signed " +
  "reported recorded measured planned approved delivered processed reviewed updated replaced introduced " +
  "higher lower stable steady strong weak new previous current next first second third final annual monthly " +
  "domestic regional central local national overall total average net gross direct indirect fixed variable " +
  "while after before during across against between within through under over about almost nearly mainly"
).split(" ");
const TITLE_WORDS = (
  "Revenue Orders Margins Costs Growth Markets Customers Service Delivery Regions Teams Products Invoicing " +
  "Receivables Payables Cash Budgets Forecasts Planning Systems Processes Suppliers Contracts Staff Offices " +
  "Branches Sales Pricing Demand Supply Expenses Income Tax Capital Investment Projects Risk Quality Audit " +
  "Controls Policy Standards Partners Logistics Procurement Treasury Compliance Facilities Training"
).split(" ");
const ACCOUNTS = ["売上高", "仕入高", "給料手当", "地代家賃", "旅費交通費", "通信費", "消耗品費", "支払手数料", "水道光熱費", "広告宣伝費", "外注費", "減価償却費"];
const PAYEES = ["Northwind", "Contoso", "Fabrikam", "Tailspin", "Litware", "Proseware", "Adatum", "Woodgrove", "みなと商事", "さくら物流", "あおば工業", "ひかり通信"];
const MEMOS = ["monthly fee", "invoice", "refund", "deposit", "transfer", "card payment", "settlement", "advance", "adjustment", "service charge"];

const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function fontFaces() {
  return [
    ["Noto Sans", "NotoSans-Regular.ttf", "truetype", 400],
    ["Noto Sans", "NotoSans-Bold.ttf", "truetype", 700],
    ["Noto Sans JP", "NotoSansJP-Regular.otf", "opentype", 400],
    ["Noto Sans JP", "NotoSansJP-Bold.otf", "opentype", 700],
  ]
    .map(([family, file, format, weight]) => `@font-face { font-family: "${family}"; src: url("${FONTS_URL}/${file}") format("${format}"); font-weight: ${weight}; font-style: normal; }`)
    .join("\n");
}

function makeText(rand) {
  const int = (lo, hi) => lo + Math.floor(rand() * (hi - lo + 1));
  const pick = (list) => list[Math.floor(rand() * list.length)];
  const sentence = () => {
    const n = int(6, 16);
    const words = [];
    for (let i = 0; i < n; i++) words.push(pick(WORDS));
    if (rand() < 0.25) words.splice(int(1, n - 1), 0, String(int(2, 980)) + (rand() < 0.5 ? "%" : ""));
    const s = words.join(" ");
    return s[0].toUpperCase() + s.slice(1) + ".";
  };
  const paragraph = () => {
    const n = int(2, 5);
    const out = [];
    for (let i = 0; i < n; i++) out.push(sentence());
    return out.join(" ");
  };
  const title = (lo, hi) => {
    const n = int(lo, hi);
    const words = [];
    for (let i = 0; i < n; i++) words.push(i === 0 ? pick(TITLE_WORDS) : pick(WORDS));
    return words.join(" ");
  };
  return { int, pick, paragraph, title };
}

export const REPORT = { chapters: 10, sections: 10, paragraphs: 50, tables: 6, tableRows: 12 };

/** The long report: 5,000 paragraphs in 10 chapters, 6 tables of figures. */
export function longReport() {
  const t = makeText(prng(20261114));
  const tableAt = new Set([1, 3, 4, 6, 8, 9].map((c) => `${c}-4`));
  const chapters = [];
  for (let c = 0; c < REPORT.chapters; c++) {
    const sections = [];
    for (let s = 0; s < REPORT.sections; s++) {
      const paras = [];
      for (let p = 0; p < REPORT.paragraphs; p++) paras.push(`<p>${esc(t.paragraph())}</p>`);
      if (tableAt.has(`${c}-${s}`)) paras.splice(10, 0, figuresTable(t));
      sections.push(`<section class="section">\n<h3>${esc(t.title(2, 5))}</h3>\n${paras.join("\n")}\n</section>`);
    }
    chapters.push(`<section class="chapter">\n<h2>${esc(t.title(2, 4))}</h2>\n${sections.join("\n")}\n</section>`);
  }
  return `<!doctype html>
<!-- A long report for the benchmark's page set. Generated by bench/pages/gen-pages.mjs; edit the generator, not this file. -->
<html lang="en">
<head>
<meta charset="utf-8">
<title>Annual operations report FY2026</title>
<style>
${fontFaces()}
@page { size: 210mm 297mm; margin: 18mm 16mm; }
html, body { margin: 0; padding: 0; background: #ffffff; color: #1f2933; }
body { font-family: "Noto Sans", "Noto Sans JP"; font-size: 14px; line-height: 20px; }
h1 { font-size: 28px; line-height: 36px; margin: 0 0 12px; }
h2 { font-size: 20px; line-height: 28px; margin: 0 0 12px; }
h3 { font-size: 16px; line-height: 22px; margin: 18px 0 8px; }
p { margin: 0 0 8px; }
.lede { font-size: 16px; line-height: 24px; color: #3e4c59; }
.chapter { break-before: page; }
#toc ol { margin: 0 0 8px; padding-left: 28px; }
#toc li { margin: 0 0 2px; }
table { border-collapse: collapse; width: 100%; margin: 8px 0 14px; font-size: 13px; line-height: 18px; }
caption { caption-side: top; text-align: left; font-weight: 700; padding: 0 0 6px; }
th, td { border: 1px solid #c9d2dc; padding: 4px 8px; text-align: left; }
thead th { background: #eef2f6; }
tfoot td { font-weight: 700; background: #f7f9fb; }
.num { text-align: right; }
</style>
</head>
<body>
<header>
<h1>Annual operations report FY2026</h1>
<p class="lede">Operations, finance and the regional offices over the fiscal year, chapter by chapter.</p>
</header>
<nav id="toc">
<h2>Contents</h2>
<ol></ol>
</nav>
<main>
${chapters.join("\n")}
</main>
<script>
(function () {
  "use strict";
  var toc = document.querySelector("#toc ol");
  var chapters = document.querySelectorAll("main > .chapter");
  for (var c = 0; c < chapters.length; c++) {
    var h2 = chapters[c].querySelector("h2");
    h2.textContent = (c + 1) + ". " + h2.textContent;
    var item = document.createElement("li");
    item.textContent = h2.textContent;
    var sub = document.createElement("ol");
    var sections = chapters[c].querySelectorAll(".section");
    for (var s = 0; s < sections.length; s++) {
      var h3 = sections[s].querySelector("h3");
      h3.textContent = (c + 1) + "." + (s + 1) + " " + h3.textContent;
      var entry = document.createElement("li");
      entry.textContent = h3.textContent;
      sub.appendChild(entry);
    }
    item.appendChild(sub);
    toc.appendChild(item);
  }
  var group = function (n) { return String(n).replace(/\\B(?=(\\d{3})+$)/g, ","); };
  var tables = document.querySelectorAll("table.figures");
  for (var i = 0; i < tables.length; i++) {
    var caption = tables[i].querySelector("caption");
    caption.textContent = "Table " + (i + 1) + ". " + caption.textContent;
    var rows = tables[i].querySelectorAll("tbody tr");
    var sums = [];
    for (var r = 0; r < rows.length; r++) {
      var cells = rows[r].querySelectorAll("td.num");
      for (var k = 0; k < cells.length; k++) {
        var v = parseInt(cells[k].textContent, 10);
        sums[k] = (sums[k] || 0) + v;
        cells[k].textContent = group(v);
      }
    }
    var totals = tables[i].querySelectorAll("tfoot td.num");
    for (var k = 0; k < totals.length; k++) totals[k].textContent = group(sums[k]);
  }
})();
</script>
</body>
</html>
`;
}

function figuresTable(t) {
  const head = ["Office", "Q1", "Q2", "Q3", "Q4", "Year"];
  const rows = [];
  for (let r = 0; r < REPORT.tableRows; r++) {
    const q = [0, 1, 2, 3].map(() => t.int(1200, 98000));
    const cells = [t.pick(TITLE_WORDS) + " " + (r + 1), ...q, q.reduce((a, b) => a + b, 0)];
    rows.push(`<tr>${cells.map((v, i) => (i === 0 ? `<td>${esc(v)}</td>` : `<td class="num">${v}</td>`)).join("")}</tr>`);
  }
  return `<table class="figures">
<caption>${esc(t.title(3, 6))}</caption>
<thead><tr>${head.map((h, i) => `<th${i ? ' class="num"' : ""}>${h}</th>`).join("")}</tr></thead>
<tbody>
${rows.join("\n")}
</tbody>
<tfoot><tr><td>Total</td>${head.slice(1).map(() => '<td class="num"></td>').join("")}</tr></tfoot>
</table>`;
}

export const LEDGER = { rows: 2000 };

/** The long table: a ledger of 2,000 entries, one row each, short cells. */
export function longTable() {
  const t = makeText(prng(20261115));
  // April 2026 to March 2027, about five or six entries a day, in date order.
  const days = [30, 31, 30, 31, 31, 30, 31, 30, 31, 31, 28, 31];
  const rows = [];
  for (let i = 0; i < LEDGER.rows; i++) {
    let day = Math.floor((i * 365) / LEDGER.rows);
    let month = 0;
    while (day >= days[month]) day -= days[month++];
    const m = ((month + 3) % 12) + 1;
    const y = month < 9 ? 2026 : 2027;
    const date = `${y}-${String(m).padStart(2, "0")}-${String(day + 1).padStart(2, "0")}`;
    const income = t.int(0, 99) < 48;
    const account = income ? "売上高" : t.pick(ACCOUNTS.slice(1));
    const amount = t.int(1, 400) * (t.int(0, 3) === 0 ? 1000 : 100);
    const memo = `${t.pick(PAYEES)} ${t.pick(MEMOS)}`;
    rows.push(
      `<tr><td class="num">${i + 1}</td><td>${date}</td><td>${account}</td><td>${esc(memo)}</td>` +
        `<td class="num">${income ? amount : ""}</td><td class="num">${income ? "" : amount}</td><td class="num"></td></tr>`,
    );
  }
  return `<!doctype html>
<!-- A long table for the benchmark's page set. Generated by bench/pages/gen-pages.mjs; edit the generator, not this file. -->
<html lang="ja">
<head>
<meta charset="utf-8">
<title>総勘定元帳 FY2026</title>
<style>
${fontFaces()}
@page { size: 210mm 297mm; margin: 15mm 12mm; }
html, body { margin: 0; padding: 0; background: #ffffff; color: #1f2933; }
body { font-family: "Noto Sans JP", "Noto Sans"; font-size: 12px; line-height: 16px; }
h1 { font-size: 22px; line-height: 30px; margin: 0 0 4px; }
p { margin: 0 0 10px; }
table { border-collapse: collapse; width: 100%; }
th, td { border-bottom: 1px solid #d5dce3; padding: 3px 6px; text-align: left; white-space: nowrap; }
thead th { background: #eef2f6; border-bottom: 1px solid #9aa5b1; }
tbody tr:nth-child(even) { background: #f7f9fb; }
tfoot td { font-weight: 700; border-top: 2px solid #52606d; }
.num { text-align: right; }
</style>
</head>
<body>
<h1>総勘定元帳 General ledger FY2026</h1>
<p>Northwind Trading 株式会社 · 2026-04-01 to 2027-03-31 · <span id="count"></span> entries</p>
<table id="ledger">
<thead><tr><th class="num">No.</th><th>日付</th><th>勘定科目</th><th>摘要</th><th class="num">入金</th><th class="num">出金</th><th class="num">残高</th></tr></thead>
<tbody>
${rows.join("\n")}
</tbody>
<tfoot><tr><td></td><td></td><td>合計</td><td></td><td class="num" id="in"></td><td class="num" id="out"></td><td class="num" id="end"></td></tr></tfoot>
</table>
<script>
(function () {
  "use strict";
  var group = function (n) { var s = String(Math.abs(n)).replace(/\\B(?=(\\d{3})+$)/g, ","); return n < 0 ? "-" + s : s; };
  var rows = document.querySelectorAll("#ledger tbody tr");
  var balance = 20000000, totalIn = 0, totalOut = 0;
  for (var i = 0; i < rows.length; i++) {
    var cells = rows[i].children;
    var credit = cells[4].textContent ? parseInt(cells[4].textContent, 10) : 0;
    var debit = cells[5].textContent ? parseInt(cells[5].textContent, 10) : 0;
    balance += credit - debit;
    totalIn += credit;
    totalOut += debit;
    if (credit) cells[4].textContent = group(credit);
    if (debit) cells[5].textContent = group(debit);
    cells[6].textContent = group(balance);
  }
  document.getElementById("count").textContent = group(rows.length);
  document.getElementById("in").textContent = group(totalIn);
  document.getElementById("out").textContent = group(totalOut);
  document.getElementById("end").textContent = group(balance);
})();
</script>
</body>
</html>
`;
}

/** The page set's generated pages: file name → HTML. */
export function generatedPages() {
  return { "long-report.html": longReport(), "long-table.html": longTable() };
}

const sha256 = (s) => crypto.createHash("sha256").update(s, "utf8").digest("hex");

export function sumsText(pages) {
  return Object.entries(pages)
    .map(([name, html]) => `${sha256(html)}  ${name}\n`)
    .join("");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { out: { type: "string" }, check: { type: "boolean", default: false }, sums: { type: "boolean", default: false } } });
  const pages = generatedPages();
  const sums = sumsText(pages);
  if (values.check) {
    const committed = fs.existsSync(SUMS) ? fs.readFileSync(SUMS, "utf8") : "";
    if (committed !== sums) {
      console.error(`bench/pages: the generator's output differs from bench/pages/SHA256SUMS\n  committed:\n${committed}  generated:\n${sums}`);
      process.exit(1);
    }
    console.log("bench/pages: the generated pages match SHA256SUMS");
  } else if (values.sums) {
    fs.writeFileSync(SUMS, sums);
    console.log(`wrote ${path.relative(process.cwd(), SUMS)}`);
  } else {
    const out = path.resolve(values.out || DEFAULT_OUT);
    fs.mkdirSync(out, { recursive: true });
    const written = {};
    for (const [name, html] of Object.entries(pages)) {
      fs.writeFileSync(path.join(out, name), html);
      written[name] = { path: path.join(out, name), bytes: Buffer.byteLength(html), sha256: sha256(html) };
    }
    console.log(JSON.stringify(written));
  }
}
