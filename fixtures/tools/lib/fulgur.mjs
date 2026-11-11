// Prints HTML with fulgur the way the fixtures need: the fixture fonts passed
// with --font, and FONTCONFIG_FILE pointing at a config that only knows
// fixtures/fonts, so no system font can reach the PDF (fulgur resolves SVG
// text through fontconfig).

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { FIXTURES_DIR } from "./fixtures.mjs";

const FONTS_DIR = path.join(FIXTURES_DIR, "fonts");

export function fulgurBinary() {
  return process.env.FULGUR || "fulgur";
}

/**
 * Which fulgur every later print uses, for a tool's --fulgur option. It is set in
 * the environment, so the tools this one starts (compare.mjs) print with it too.
 */
export function useFulgur(bin) {
  if (bin) process.env.FULGUR = bin.includes("/") ? path.resolve(bin) : bin;
}

export function fulgurVersion() {
  const r = spawnSync(fulgurBinary(), ["--version"], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`fulgur not found (set FULGUR or: cargo install fulgur-cli --version =0.40.0 --locked)`);
  return r.stdout.trim().replace(/^fulgur\s+/, "");
}

function fontconfig(workDir) {
  const conf = path.join(workDir, "fonts.conf");
  if (!fs.existsSync(conf)) {
    fs.mkdirSync(workDir, { recursive: true });
    fs.writeFileSync(
      conf,
      `<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>${FONTS_DIR}</dir>
  <cachedir>${path.join(workDir, "fontconfig-cache")}</cachedir>
</fontconfig>
`,
    );
  }
  return conf;
}

/** The fixture fonts and `extraFonts` as --font arguments. */
function fontArgs(extraFonts) {
  return fs
    .readdirSync(FONTS_DIR)
    .filter((f) => /\.(ttf|otf)$/i.test(f))
    .sort()
    .flatMap((f) => ["--font", path.join(FONTS_DIR, f)])
    .concat(extraFonts.flatMap((f) => ["--font", f]));
}

/** The one command every print and every measurement of a print runs: { bin, args, env }. */
export function fulgurCommand(htmlPath, pdfPath, workDir, extraFonts = []) {
  return {
    bin: fulgurBinary(),
    args: ["render", ...fontArgs(extraFonts), "--creation-date", "2026-01-01", "-o", pdfPath, htmlPath],
    env: { ...process.env, FONTCONFIG_FILE: fontconfig(workDir) },
  };
}

/** A printer that hasn't finished a fixture page by then hangs. */
export const PRINT_TIMEOUT_MS = 120_000;

/**
 * fulgur render <html> -o <pdf> with only the fixture fonts, and `extraFonts` for a page
 * with its own: fulgur 0.40.0 doesn't load @font-face URLs. Throws with fulgur's output on
 * failure; the error's `printer` says how it ended ({ status, signal, timed_out }).
 */
export function fulgurPrint(htmlPath, pdfPath, workDir, extraFonts = []) {
  const { bin, args, env } = fulgurCommand(htmlPath, pdfPath, workDir, extraFonts);
  const r = spawnSync(bin, args, { encoding: "utf8", env, timeout: PRINT_TIMEOUT_MS, killSignal: "SIGKILL" });
  if (r.status !== 0) {
    const timedOut = r.error?.code === "ETIMEDOUT";
    const how = timedOut ? `didn't finish in ${PRINT_TIMEOUT_MS / 1000} s` : r.signal ? `was killed by ${r.signal}` : `exited ${r.status}`;
    const e = new Error(`fulgur ${how} on ${htmlPath}:\n${`${r.stdout ?? ""}${r.stderr ?? ""}`.split("\n").slice(0, 30).join("\n")}`);
    e.printer = { status: r.status, signal: r.signal, timed_out: timedOut };
    throw e;
  }
}

/** A printed PDF's size and hash: two printers that write the same bytes printed the same. */
export function pdfIdentity(pdfPath) {
  const data = fs.readFileSync(pdfPath);
  return { bytes: data.length, sha256: crypto.createHash("sha256").update(data).digest("hex") };
}

const MEASURE = path.resolve(FIXTURES_DIR, "..", "bench", "tools", "measure.py");
/** A print lasts about 50 ms: the benchmark's 10 ms sampler would see it a handful of times. */
export const MEASURE_INTERVAL_MS = 1;

/**
 * Time and peak memory of the print fulgurPrint makes, by bench/tools/measure.py:
 * wall clock from spawn to exit (median of `runs` unsampled runs) and the peak
 * of summed PSS over the process tree (`runs` sampled runs), as the Grover
 * benchmark measures a cold run, but sampled every MEASURE_INTERVAL_MS. Returns its summary without the single runs,
 * or { error } when a run failed: a number is measured or absent, never estimated.
 */
export function fulgurMeasure(htmlPath, pdfPath, workDir, extraFonts = [], runs = 5) {
  const { bin, args, env } = fulgurCommand(htmlPath, pdfPath, workDir, extraFonts);
  const r = spawnSync("python3", ["-I", MEASURE, "--runs", String(runs), "--mem-runs", String(runs), "--interval-ms", String(MEASURE_INTERVAL_MS), "--timeout", String(PRINT_TIMEOUT_MS / 1000), "--", bin, ...args], { encoding: "utf8", env, maxBuffer: 1 << 26 });
  let out;
  try {
    out = JSON.parse(r.stdout);
  } catch {
    return { error: `measure.py exited ${r.status}: ${`${r.stderr}`.split("\n").slice(-5).join("\n")}` };
  }
  if (r.status !== 0) return { error: out.error ?? `measure.py exited ${r.status}`, status: out.status ?? null, signal: out.signal ?? null, timed_out: out.timed_out ?? false };
  const { runs: _single, ...summary } = out;
  return { runs, ...summary };
}
