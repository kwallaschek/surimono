// Prints HTML with fulgur the way the fixtures need: the fixture fonts passed
// with --font, and FONTCONFIG_FILE pointing at a config that only knows
// fixtures/fonts, so no system font can reach the PDF (fulgur resolves SVG
// text through fontconfig).

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { FIXTURES_DIR } from "./fixtures.mjs";

const FONTS_DIR = path.join(FIXTURES_DIR, "fonts");

export function fulgurBinary() {
  return process.env.FULGUR || "fulgur";
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

/** fulgur render <html> -o <pdf> with only the fixture fonts. Throws with fulgur's output on failure. */
export function fulgurPrint(htmlPath, pdfPath, workDir) {
  const fonts = fs
    .readdirSync(FONTS_DIR)
    .filter((f) => /\.(ttf|otf)$/i.test(f))
    .sort()
    .flatMap((f) => ["--font", path.join(FONTS_DIR, f)]);
  const r = spawnSync(fulgurBinary(), ["render", ...fonts, "--creation-date", "2026-01-01", "-o", pdfPath, htmlPath], {
    encoding: "utf8",
    env: { ...process.env, FONTCONFIG_FILE: fontconfig(workDir) },
  });
  if (r.status !== 0) throw new Error(`fulgur failed on ${htmlPath}:\n${`${r.stdout}${r.stderr}`.split("\n").slice(0, 30).join("\n")}`);
}
