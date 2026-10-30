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
export const PASS_WORST_WINDOW = 0.15;

export const METRIC =
  "near-match (1 px radius, per-channel tolerance 96/255), worst 32x32 window's share of mismatched ink; pixelmatch 8.0.0 (threshold 0.1, AA ignored) reported as diff_ratio_ink";

/** Committed Chrome references of a fixture: full pages and single islands. */
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
  return { pages: list(/^chrome-(\d+)\.png$/), islands: list(/^island-(\d+)\.png$/) };
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
    const pass = score.size_match && score.worst_window <= PASS_WORST_WINDOW;
    const sbs = path.join(outDir, `side-by-side-${i + 1}.png`);
    writePng(sbs, sideBySide([expected, actual, image]));
    pages.push({ page: i + 1, pass, ...score, side_by_side: path.basename(sbs) });
  }
  const result = {
    subject,
    pdf,
    label,
    metric: METRIC,
    pixel_threshold: PIXEL_THRESHOLD,
    pass_worst_window: PASS_WORST_WINDOW,
    poppler: popplerVersion(),
    dpi: DPI,
    page_count: { chrome: expectedPngs.length, [label]: actualPages.length },
    pass: pages.every((p) => p.pass),
    worst_window: Math.max(...pages.map((p) => p.worst_window ?? 1)),
    worst_diff_ratio_ink: Math.max(...pages.map((p) => p.diff_ratio_ink ?? 1)),
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
        : `worst window ${(p.worst_window * 100).toFixed(1)}% at ${p.worst_window_at}, near-mismatch ${(p.near_ink_share * 100).toFixed(2)}% of ink, pixelmatch ${(p.diff_ratio_ink * 100).toFixed(2)}%${p.size_match ? "" : `, size ${p.actual_size} vs Chrome ${p.expected_size}`}`;
    console.log(`${p.pass ? "pass" : "FAIL"} ${result.subject}${result.pages.length > 1 ? ` page ${p.page}` : ""}: ${s}`);
  }
}
