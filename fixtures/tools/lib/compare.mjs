// Page-by-page comparison of a PDF against committed Chrome reference PNGs.

import fs from "node:fs";
import path from "node:path";
import { DPI, popplerVersion, rasterise } from "./fixtures.mjs";
import { PIXEL_THRESHOLD, diff, readPng, sideBySide, writePng } from "./png.mjs";

/**
 * A page or island passes when no 32x32 window has more than this share of
 * its ink near-mismatched (lib/png.mjs). Chosen with calibrate.mjs; see
 * fixtures/README.md ("Diff metric").
 */
export const PASS_WORST_WINDOW = 0.1;
/** Colour checks (lib/png.mjs): worst window's share of solid pixels in the wrong colour, and of inked pixels in the wrong hue. */
export const PASS_COLOUR_WINDOW = 0.1;
export const PASS_HUE_WINDOW = 0.3;

/** Whether a page or island score passes every image check; names the failed ones. */
export function verdictOf(score) {
  const failed = [];
  if (!score.size_match) failed.push("size");
  if (score.worst_window > PASS_WORST_WINDOW) failed.push("shape");
  if (score.colour_worst_window > PASS_COLOUR_WINDOW) failed.push("colour");
  if (score.hue_worst_window > PASS_HUE_WINDOW) failed.push("hue");
  return { pass: failed.length === 0, failed };
}

export const METRIC =
  "shape: near-match (1 px radius, per-channel tolerance 128/255), worst 32x32 window's share of mismatched ink, pass <= 10%; colour: solid pixels (flat 5x5) within 16/255 per channel at 1 px, worst window <= 10%; hue: ink direction of strongly inked pixels within 8 degrees at 1 px, worst window <= 30%; pixelmatch 8.0.0 (threshold 0.1, AA ignored) reported as diff_ratio_ink";

/**
 * Committed Chrome references of a fixture: full pages, single islands, and the
 * strings in each island's SVG (island-<i>.texts.json, from lib/svgtext.mjs).
 */
export function referenceFiles(fixtureDir) {
  const refDir = path.join(fixtureDir, "reference");
  const list = (re) =>
    fs.existsSync(refDir)
      ? fs
          .readdirSync(refDir)
          .filter((f) => re.test(f))
          .sort((a, b) => Number(re.exec(a)[1]) - Number(re.exec(b)[1]))
          .map((f) => path.join(refDir, f))
      : [];
  return { pages: list(/^chrome-(\d+)\.png$/), islands: list(/^island-(\d+)\.png$/), texts: list(/^island-(\d+)\.texts\.json$/) };
}

/**
 * Rasterises `pdf` into `outDir` as `<label>-<n>.png` and diffs each page with
 * `expectedPngs`. Writes side-by-side-<n>.png (Chrome | label | diff) and
 * returns the result object (also written to outDir/score.json).
 */
export function comparePdf({ expectedPngs, pdf, outDir, label, subject }) {
  fs.mkdirSync(outDir, { recursive: true });
  const actualPages = rasterise(pdf, outDir, label);
  const pages = [];
  const n = Math.max(expectedPngs.length, actualPages.length);
  for (let i = 0; i < n; i++) {
    if (i >= expectedPngs.length || i >= actualPages.length) {
      pages.push({ page: i + 1, pass: false, detail: i >= expectedPngs.length ? "extra page, not in Chrome" : "page missing, Chrome has it" });
      continue;
    }
    const expected = readPng(expectedPngs[i]);
    const actual = readPng(actualPages[i]);
    const { score, image } = diff(expected, actual);
    const { pass, failed } = verdictOf(score);
    const sbs = path.join(outDir, `side-by-side-${i + 1}.png`);
    writePng(sbs, sideBySide([expected, actual, image]));
    pages.push({ page: i + 1, pass, failed, ...score, side_by_side: path.basename(sbs) });
  }
  const result = {
    subject,
    pdf,
    label,
    metric: METRIC,
    pixel_threshold: PIXEL_THRESHOLD,
    pass_worst_window: PASS_WORST_WINDOW,
    pass_colour_window: PASS_COLOUR_WINDOW,
    pass_hue_window: PASS_HUE_WINDOW,
    poppler: popplerVersion(),
    dpi: DPI,
    page_count: { chrome: expectedPngs.length, [label]: actualPages.length },
    pass: pages.every((p) => p.pass),
    worst_window: Math.max(...pages.map((p) => p.worst_window ?? 1)),
    worst_diff_ratio_ink: Math.max(...pages.map((p) => p.diff_ratio_ink ?? 1)),
    worst_colour_window: Math.max(...pages.map((p) => p.colour_worst_window ?? 1)),
    worst_hue_window: Math.max(...pages.map((p) => p.hue_worst_window ?? 1)),
    pages,
  };
  fs.writeFileSync(path.join(outDir, "score.json"), `${JSON.stringify(result, null, 2)}\n`);
  return result;
}

export function printResult(result) {
  for (const p of result.pages) {
    const s =
      p.worst_window === undefined
        ? p.detail
        : `worst window ${(p.worst_window * 100).toFixed(1)}% at ${p.worst_window_at}, colour ${(p.colour_worst_window * 100).toFixed(1)}%, hue ${(p.hue_worst_window * 100).toFixed(1)}%, near-mismatch ${(p.near_ink_share * 100).toFixed(2)}% of ink, pixelmatch ${(p.diff_ratio_ink * 100).toFixed(2)}%${p.size_match ? "" : `, size ${p.actual_size} vs Chrome ${p.expected_size}`}`;
    console.log(`${p.pass ? "pass" : `FAIL(${(p.failed ?? []).join("+") || "missing"})`} ${result.subject}${result.pages.length > 1 ? ` page ${p.page}` : ""}: ${s}`);
  }
  if (result.text) {
    const t = result.text;
    console.log(
      `${t.pass ? "pass" : "FAIL(text)"} ${result.subject} text: ${t.reference_strings} strings in Chrome's SVG, found ${t.candidate_text} as <text> and ${t.candidate_outlined} as outline labels${t.missing.length ? `; missing ${JSON.stringify(t.missing.slice(0, 10))}` : ""}${t.extra.length ? `; extra ${JSON.stringify(t.extra.slice(0, 10))}` : ""}`,
    );
  }
  if (result.font) {
    const f = result.font;
    const shown = f.mismatches.slice(0, 5).map((x) => `${JSON.stringify(x.key)}: Chrome ${x.chrome}, surimono ${x.surimono}`);
    const lacking = (f.missing ?? []).slice(0, 5).map((x) => JSON.stringify(x.key));
    console.log(
      `${f.pass ? "pass" : "FAIL(font)"} ${result.subject} declared font weight and style: ${f.compared} texts compared${shown.length ? `; ${shown.join("; ")}` : ""}${lacking.length ? `; no text or outline for ${lacking.join(", ")}` : ""}`,
    );
  }
}
