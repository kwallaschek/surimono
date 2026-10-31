// Shared pieces of the fixture pipeline: fixture discovery, the pinned tool
// versions, the offline page server inside Playwright, Chrome rendering of
// islands, and rasterising PDFs with pdftoppm.

import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
export const TOOLS_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const FIXTURES_DIR = path.resolve(TOOLS_DIR, "..");

/** Pages are served from this made-up origin; nothing else is reachable. */
export const ORIGIN = "http://fixtures.surimono.test";
/** Raster resolution for every PDF page (CSS px = device px at 96 dpi). */
export const DPI = 96;
/** Every Chrome render runs in this time zone and locale (see fixtures/README.md). */
export const TIMEZONE = "UTC";
export const LOCALE = "en-US";

const MARKER_RE = /<script\b[^>]*\btype\s*=\s*["']application\/vnd\.surimono\+json["'][^>]*>[\s\S]*?<\/script\s*>/gi;

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".ttf": "font/ttf",
  ".otf": "font/otf",
  ".woff2": "font/woff2",
};

export function pkgVersion(name) {
  // Read from disk: some packages (vega) do not export ./package.json.
  return JSON.parse(fs.readFileSync(path.join(TOOLS_DIR, "node_modules", name, "package.json"), "utf8")).version;
}

/** Versions pinned in package.json; the lockfile pins their integrity. */
export function pinnedVersions() {
  const pkg = JSON.parse(fs.readFileSync(path.join(TOOLS_DIR, "package.json"), "utf8"));
  return pkg.dependencies;
}

/** Fails loudly when node_modules does not match package.json. */
export function checkInstalledVersions() {
  const pinned = pinnedVersions();
  const wrong = Object.entries(pinned).filter(([name, v]) => pkgVersion(name) !== v);
  if (wrong.length) {
    throw new Error(`installed versions differ from package.json: ${wrong.map(([n, v]) => `${n} ${pkgVersion(n)} (want ${v})`).join(", ")}; run npm ci in fixtures/tools`);
  }
  return pinned;
}

/** The poppler version of `pdftoppm` (it prints the version on stderr). */
export function popplerVersion() {
  const r = spawnSync("pdftoppm", ["-v"], { encoding: "utf8" });
  const m = /pdftoppm version ([\d.]+)/.exec(`${r.stdout}${r.stderr}`);
  if (!m) throw new Error("pdftoppm not found (install poppler-utils)");
  return m[1];
}

/** All island fixtures (folders with an input.html), sorted by name. */
export function listFixtures(kind = "islands", only = []) {
  const root = path.join(FIXTURES_DIR, kind);
  if (!fs.existsSync(root)) return [];
  const names = fs
    .readdirSync(root, { withFileTypes: true })
    .filter((d) => d.isDirectory() && fs.existsSync(path.join(root, d.name, "input.html")))
    .map((d) => d.name)
    .sort();
  const missing = only.filter((n) => !names.includes(n));
  if (missing.length) throw new Error(`no such fixture: ${missing.join(", ")}`);
  return (only.length ? names.filter((n) => only.includes(n)) : names).map((name) => ({
    name,
    kind,
    dir: path.join(root, name),
    url: `${ORIGIN}/${kind}/${name}/input.html`,
  }));
}

/**
 * Serves fixtures/ at ORIGIN from disk and aborts every other request, so a
 * fixture can never reach the network.
 */
export async function routeOffline(context, virtualPages = new Map()) {
  await context.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort("blockedbyclient");
    if (virtualPages.has(url.pathname)) {
      return route.fulfill({ status: 200, body: virtualPages.get(url.pathname), headers: { "content-type": MIME[".html"] } });
    }
    const file = path.resolve(FIXTURES_DIR, `.${decodeURIComponent(url.pathname)}`);
    if (!file.startsWith(FIXTURES_DIR + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
      return route.fulfill({ status: 404, body: "not found" });
    }
    return route.fulfill({
      status: 200,
      body: fs.readFileSync(file),
      headers: { "content-type": MIME[path.extname(file).toLowerCase()] || "application/octet-stream" },
    });
  });
}

/**
 * Launches the pinned Playwright Chromium: the full browser in new headless
 * mode (channel "chromium"), not chromium-headless-shell, so references come
 * from the same engine build a desktop Chrome or Puppeteer would use.
 */
export async function launchChromium() {
  const { chromium } = require("playwright");
  return chromium.launch({ channel: "chromium" });
}

/** The @font-face rules every fixture page uses, with `fontsUrl` pointing at fixtures/fonts. */
export function fontFaceCss(fontsUrl) {
  return [
    ["Noto Sans", "NotoSans-Regular.ttf", "truetype", 400],
    ["Noto Sans", "NotoSans-Bold.ttf", "truetype", 700],
    ["Noto Sans JP", "NotoSansJP-Regular.otf", "opentype", 400],
    ["Noto Sans JP", "NotoSansJP-Bold.otf", "opentype", 700],
  ]
    .map(([family, file, format, weight]) => `@font-face { font-family: "${family}"; src: url("${fontsUrl}/${file}") format("${format}"); font-weight: ${weight}; font-style: normal; }`)
    .join("\n");
}

/**
 * A page that holds one island SVG and nothing else, sized exactly W x H with
 * no margin. Chrome and fulgur both print it, so an island can be compared
 * without the rest of the page's layout. Fonts resolve like the fixture's own
 * (the page is served from the fixture's folder).
 */
export function islandPageHtml(svg, width, height) {
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
${fontFaceCss("../../fonts")}
@page { size: ${width}px ${height}px; margin: 0; }
html, body { margin: 0; padding: 0; background: #ffffff; }
svg { display: block; }
</style>
</head>
<body>${svg}</body>
</html>
`;
}

async function loadFontsAndPrint(page, pdfPath) {
  await page.evaluate(async () => {
    await Promise.all([...document.fonts].map((f) => f.load()));
    await document.fonts.ready;
  });
  await page.emulateMedia({ media: "print" });
  await page.pdf({ path: pdfPath, preferCSSPageSize: true, printBackground: true });
}

function newContext(browser) {
  return browser.newContext({ timezoneId: TIMEZONE, locale: LOCALE, deviceScaleFactor: 1 });
}

/** Prints one island SVG alone (islandPageHtml) in Chrome. */
export async function printIslandInChrome(browser, fixture, index, svg, width, height, pdfPath) {
  const context = await newContext(browser);
  try {
    const pagePath = `/${fixture.kind}/${fixture.name}/__island-${index}.html`;
    await routeOffline(context, new Map([[pagePath, islandPageHtml(svg, width, height)]]));
    const page = await context.newPage();
    const response = await page.goto(`${ORIGIN}${pagePath}`, { waitUntil: "load" });
    if (!response || !response.ok()) throw new Error(`could not load island page ${pagePath}`);
    await loadFontsAndPrint(page, pdfPath);
  } finally {
    await context.close();
  }
}

/** Library bundles injected into the page, in load order. */
function libraryScripts() {
  const nm = path.join(TOOLS_DIR, "node_modules");
  return [
    path.join(nm, "echarts", "dist", "echarts.min.js"),
    path.join(nm, "vega", "build", "vega.min.js"),
    path.join(nm, "vega-lite", "build", "vega-lite.min.js"),
    path.join(TOOLS_DIR, "lib", "islands-browser.js"),
  ];
}

/**
 * Renders one fixture in Chrome: islands drawn client-side by the real
 * libraries, then the page printed to PDF. Returns the island SVGs and fonts.
 */
export async function renderFixtureInChrome(browser, fixture, pdfPath) {
  const context = await newContext(browser);
  const messages = [];
  try {
    await routeOffline(context);
    const page = await context.newPage();
    page.on("console", (m) => messages.push(`${m.type()}: ${m.text()}`));
    page.on("pageerror", (e) => messages.push(`pageerror: ${e.message}`));
    const response = await page.goto(fixture.url, { waitUntil: "load" });
    if (!response || !response.ok()) throw new Error(`${fixture.name}: could not load ${fixture.url}`);
    for (const script of libraryScripts()) await page.addScriptTag({ path: script });
    const result = await page.evaluate(() => window.__surimonoRenderIslands());
    await loadFontsAndPrint(page, pdfPath);
    return { ...result, console: messages };
  } catch (e) {
    e.message = `${fixture.name}: ${e.message}${messages.length ? `\nconsole:\n  ${messages.join("\n  ")}` : ""}`;
    throw e;
  } finally {
    await context.close();
  }
}

/** An attribute's value as written in a start tag (double, single or no quotes), or undefined. */
function rawAttribute(startTag, name) {
  const m = new RegExp(`\\s${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>"']+))`, "i").exec(startTag);
  return m ? (m[1] ?? m[2] ?? m[3]) : undefined;
}

/**
 * What surimono's pre-pass puts where a marker was (crates/surimono/src/prepass.rs):
 * <div class="surimono-island[ marker classes]"[ id="…"] data-renderer="…">SVG</div>,
 * attribute values copied as written.
 */
export function islandWrapper(markerHtml, svg) {
  const start = /^<script\b[^>]*>/i.exec(markerHtml)[0];
  const q = (v) => v.replace(/"/g, "&quot;");
  const cls = rawAttribute(start, "class")?.trim();
  const id = rawAttribute(start, "id");
  const renderer = rawAttribute(start, "data-renderer")?.trim();
  return `<div class="surimono-island${cls ? ` ${q(cls)}` : ""}"${id ? ` id="${q(id)}"` : ""}${renderer ? ` data-renderer="${q(renderer)}"` : ""}>${svg}</div>`;
}

/**
 * The page with each island marker replaced by its SVG in surimono's wrapper
 * (islandWrapper), everything else byte for byte unchanged: what a perfect
 * surimono pre-pass would output.
 */
export function prepassHtml(sourceHtml, svgs) {
  let i = 0;
  const html = sourceHtml.replace(MARKER_RE, (m) => {
    if (i >= svgs.length) throw new Error("more markers in the source than islands rendered");
    return islandWrapper(m, svgs[i++]);
  });
  if (i !== svgs.length) throw new Error(`source has ${i} markers, Chrome rendered ${svgs.length} islands`);
  return html;
}

/**
 * The inverse of prepassHtml for surimono's real output: the island SVGs in
 * document order, cut out of each div.surimono-island. An island surimono
 * replaced with its error placeholder (class surimono-island-error, a
 * <!-- surimono: … --> comment) comes back as { error } instead of { svg }.
 */
export function islandsOf(html) {
  return [...html.matchAll(/<div class="surimono-island(?: ([^"]*))?"[^>]*>([\s\S]*?)<\/div>/g)].map((m) => {
    if ((m[1] ?? "").split(/\s+/).includes("surimono-island-error")) {
      const note = /<!--\s*surimono:\s*([\s\S]*?)\s*-->/.exec(m[2]);
      return { error: `surimono left an error placeholder: ${note ? note[1] : "no detail"}` };
    }
    return { svg: m[2] };
  });
}

/**
 * Chrome's computed font-weight and font-style of every <text> in `svg` (not
 * nested in another <text>), as [{ key, weight, style }] keyed like
 * svgTextStyles: the aria-label, else the content without <title>/<desc>,
 * whitespace removed. The outside truth reference.mjs checks svgTextStyles
 * against.
 */
export async function computedTextStyles(browser, svg) {
  const page = await browser.newPage();
  try {
    await page.setContent(`<!doctype html><html><head><style>html,body{margin:0}svg{display:block}</style></head><body><div class="surimono-island">${svg}</div></body></html>`);
    return await page.evaluate(() =>
      [...document.querySelectorAll("svg text")]
        .filter((t) => !t.parentElement.closest("text"))
        .map((t) => {
          const cs = getComputedStyle(t);
          const label = t.getAttribute("aria-label");
          const text = (n) => (n.nodeType === 3 ? n.data : n.nodeType === 1 && !/^(title|desc)$/i.test(n.nodeName) ? [...n.childNodes].map(text).join("") : "");
          const key = (label ?? [...t.childNodes].map(text).join("")).replace(/\s+/g, "");
          return { key, weight: Number(cs.fontWeight), style: cs.fontStyle };
        })
        .filter((x) => x.key),
    );
  } finally {
    await page.close();
  }
}

/** Rasterises every page of `pdf` to `<outDir>/<prefix>-<page>.png`; returns the paths. */
export function rasterise(pdf, outDir, prefix) {
  fs.mkdirSync(outDir, { recursive: true });
  const tmp = fs.mkdtempSync(path.join(outDir, `.raster-${prefix}-`));
  try {
    execFileSync("pdftoppm", ["-r", String(DPI), "-png", pdf, path.join(tmp, "p")], { stdio: ["ignore", "inherit", "inherit"] });
    const pages = fs
      .readdirSync(tmp)
      .map((f) => ({ f, n: Number(/^p-(\d+)\.png$/.exec(f)?.[1]) }))
      .filter((p) => p.n > 0)
      .sort((a, b) => a.n - b.n);
    if (!pages.length) throw new Error(`pdftoppm produced no pages for ${pdf}`);
    return pages.map(({ f, n }) => {
      const dest = path.join(outDir, `${prefix}-${n}.png`);
      fs.renameSync(path.join(tmp, f), dest);
      return dest;
    });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}
