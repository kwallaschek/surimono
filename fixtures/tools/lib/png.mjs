// PNG helpers for the fixture pipeline: read, diff, side-by-side.
//
// The score is a "near-match" diff, chosen by calibrate.mjs (fixtures/README.md,
// "Diff metric"): a pixel matches when the other image has a pixel within
// NEAR_RADIUS px whose every channel is within NEAR_TOLERANCE, checked both
// ways. Mismatched pixels are counted in 32x32 windows (stride 16), and the
// score is the worst window's share of mismatched ink. Sub-pixel text shifts
// stay near 0; a missing label, a 3 px shift, a wrong font or a resized
// chart light up one window. pixelmatch (pinned) is also reported and draws
// the diff image.
//
// The near-match tolerance is colour-blind on purpose (it absorbs
// anti-aliasing), so a second check looks at colour: "solid" pixels, whose
// 5x5 neighbourhood is one flat colour (bar, slice and legend-swatch
// interiors, table backgrounds; never thin lines or text at 96 dpi), must find
// a pixel within 1 px whose every channel is within COLOUR_TOLERANCE. The
// score is again the worst 32x32 window's share of mismatched solid pixels.
// Thin strokes (lines, text) are never solid, so their colour is checked by
// hue: on a white page an anti-aliased pixel is coverage x colour + (1 -
// coverage) x white, so the direction of (255 - R, 255 - G, 255 - B) is the
// stroke colour's whatever the coverage. Every strongly inked pixel that has
// inked pixels within 1 px in the other image needs one of them to have an ink
// direction within HUE_TOLERANCE_DEG (no inked partner at all is a position
// difference, which near-match judges).

import fs from "node:fs";
import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";

/** pixelmatch per-pixel threshold (OKLab HyAB distance, 0..1). Library default. */
export const PIXEL_THRESHOLD = 0.1;
/** Near-match: search radius in px and per-channel tolerance (0..255). */
export const NEAR_RADIUS = 1;
export const NEAR_TOLERANCE = 128;
/** Near-match: window size in px (stride is half) and the least ink a window needs to count. */
export const WINDOW = 32;
const WINDOW_MIN_INK = 24;
/** A pixel counts as ink when any channel is darker than this (white page = 255). */
const INK_LEVEL = 250;
const GAP = 16;
/** Solid colour check: a pixel is solid when every pixel within SOLID_RADIUS is within SOLID_UNIFORMITY of it. */
export const SOLID_RADIUS = 2;
export const SOLID_UNIFORMITY = 6;
/**
 * Per-channel tolerance (0..255) for solid pixels. The closest pair of fixture
 * palette colours differs by 44; fulgur and Chrome print the same fill colours
 * with no solid pixel off by more than 12 (calibrate.mjs, fixtures/README.md).
 */
export const COLOUR_TOLERANCE = 16;
/** A window counts for the colour score when it holds at least this many solid pixels. */
const COLOUR_MIN_SOLID = 64;
/** Hue check: pixels inked at least this deep (max of 255 - channel) are checked; partners need half of it. */
const HUE_MIN_DEPTH = 64;
/**
 * Largest angle between two ink directions that still counts as the same colour.
 * The closest pairs of different hues in the fixture palette are 11.3 degrees
 * apart (#2f6fb0/#4bb3c4, #e07b39/#d4a72c), and harmless shifts stay at or under
 * 9.1% of a window at 8 (calibrate.mjs, fixtures/README.md).
 */
export const HUE_TOLERANCE_DEG = 8;
/** A window counts for the hue score when it holds at least this many checked pixels. */
const HUE_MIN_PIXELS = 24;

export function readPng(path) {
  return PNG.sync.read(fs.readFileSync(path));
}

export function writePng(path, png) {
  fs.writeFileSync(path, PNG.sync.write(png));
}

/** Copies `src` onto a white canvas of `width` x `height` at (0, 0). */
function padTo(src, width, height) {
  if (src.width === width && src.height === height) return src;
  const out = new PNG({ width, height });
  out.data.fill(255);
  PNG.bitblt(src, out, 0, 0, Math.min(src.width, width), Math.min(src.height, height), 0, 0);
  return out;
}

function countInk(png) {
  let ink = 0;
  const d = png.data;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i] < INK_LEVEL || d[i + 1] < INK_LEVEL || d[i + 2] < INK_LEVEL) ink++;
  }
  return ink;
}

/** Pixels that are ink in either image (the area where a difference can matter). */
function countInkUnion(a, b) {
  let ink = 0;
  const x = a.data;
  const y = b.data;
  for (let i = 0; i < x.length; i += 4) {
    if (x[i] < INK_LEVEL || x[i + 1] < INK_LEVEL || x[i + 2] < INK_LEVEL || y[i] < INK_LEVEL || y[i + 1] < INK_LEVEL || y[i + 2] < INK_LEVEL) ink++;
  }
  return ink;
}

/** True when `src` at (x, y) has a pixel in `dst` within `radius` px whose every channel is within `tolerance`. */
function nearPixel(src, dst, w, h, x, y, radius, tolerance) {
  const i = (y * w + x) * 4;
  for (let dy = -radius; dy <= radius; dy++) {
    const yy = y + dy;
    if (yy < 0 || yy >= h) continue;
    for (let dx = -radius; dx <= radius; dx++) {
      const xx = x + dx;
      if (xx < 0 || xx >= w) continue;
      const j = (yy * w + xx) * 4;
      if (Math.abs(src[i] - dst[j]) <= tolerance && Math.abs(src[i + 1] - dst[j + 1]) <= tolerance && Math.abs(src[i + 2] - dst[j + 2]) <= tolerance) return true;
    }
  }
  return false;
}

/** Worst WINDOW x WINDOW window (stride WINDOW/2) by share of `bad` among `count`, over windows with at least `minCount`. */
function worstWindow(count, bad, w, h, minCount) {
  let worst = 0;
  let worstAt = null;
  const step = WINDOW / 2;
  for (let y0 = 0; y0 < h; y0 += step) {
    for (let x0 = 0; x0 < w; x0 += step) {
      let n = 0;
      let m = 0;
      for (let y = y0; y < Math.min(h, y0 + WINDOW); y++) {
        for (let x = x0; x < Math.min(w, x0 + WINDOW); x++) {
          n += count[y * w + x];
          m += bad[y * w + x];
        }
      }
      if (n >= minCount && m / n > worst) {
        worst = m / n;
        worstAt = [x0, y0];
      }
    }
  }
  return { worst, worstAt };
}

/** Pixels of `img` that are ink and sit in a flat SOLID_RADIUS neighbourhood. */
export function solidMask(img) {
  const { width: w, height: h, data: d } = img;
  const mask = new Uint8Array(w * h);
  const R = SOLID_RADIUS;
  const U = SOLID_UNIFORMITY;
  for (let y = R; y < h - R; y++) {
    for (let x = R; x < w - R; x++) {
      const i = (y * w + x) * 4;
      if (!(d[i] < INK_LEVEL || d[i + 1] < INK_LEVEL || d[i + 2] < INK_LEVEL)) continue;
      let flat = true;
      for (let dy = -R; dy <= R && flat; dy++) {
        for (let dx = -R; dx <= R; dx++) {
          const j = ((y + dy) * w + x + dx) * 4;
          if (Math.abs(d[j] - d[i]) > U || Math.abs(d[j + 1] - d[i + 1]) > U || Math.abs(d[j + 2] - d[i + 2]) > U) {
            flat = false;
            break;
          }
        }
      }
      if (flat) mask[y * w + x] = 1;
    }
  }
  return mask;
}

/**
 * Colour check of two same-sized images: every solid pixel of either image
 * needs a pixel within 1 px of the same colour (each channel within
 * `tolerance`) in the other. Catches swapped or wrong series colours, which
 * the near-match tolerance of 128 lets through.
 */
export function colourMatch(a, b, { tolerance = COLOUR_TOLERANCE } = {}) {
  const { width: w, height: h } = a;
  const sa = solidMask(a);
  const sb = solidMask(b);
  const solid = new Uint8Array(w * h);
  const bad = new Uint8Array(w * h);
  let solidN = 0;
  let badN = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const p = y * w + x;
      if (!sa[p] && !sb[p]) continue;
      solid[p] = 1;
      solidN++;
      if ((sa[p] && !nearPixel(a.data, b.data, w, h, x, y, 1, tolerance)) || (sb[p] && !nearPixel(b.data, a.data, w, h, x, y, 1, tolerance))) {
        bad[p] = 1;
        badN++;
      }
    }
  }
  const { worst, worstAt } = worstWindow(solid, bad, w, h, COLOUR_MIN_SOLID);
  return { mismatch: bad, solid_pixels: solidN, colour_mismatch_pixels: badN, colour_worst_window: worst, colour_worst_window_at: worstAt };
}

/**
 * Hue check of two same-sized images: every pixel inked at least HUE_MIN_DEPTH
 * deep in either image needs a pixel within 1 px in the other whose ink
 * direction (255 - R, 255 - G, 255 - B) is within `degrees`. Catches a line or
 * label drawn in the wrong colour, which neither near-match nor the solid
 * check sees.
 */
export function hueMatch(a, b, { degrees = HUE_TOLERANCE_DEG } = {}) {
  const { width: w, height: h } = a;
  const cos = Math.cos((degrees * Math.PI) / 180);
  const dir = (d) => {
    const n = w * h;
    const v = new Float32Array(n * 3);
    const depth = new Uint8Array(n);
    for (let p = 0; p < n; p++) {
      const r = 255 - d[p * 4];
      const g = 255 - d[p * 4 + 1];
      const b2 = 255 - d[p * 4 + 2];
      const len = Math.hypot(r, g, b2);
      depth[p] = Math.max(r, g, b2);
      if (len > 0) {
        v[p * 3] = r / len;
        v[p * 3 + 1] = g / len;
        v[p * 3 + 2] = b2 / len;
      }
    }
    return { v, depth };
  };
  const A = dir(a.data);
  const B = dir(b.data);
  // 1: a partner within 1 px has the same hue; 0: partners exist, none has it;
  // -1: no inked partner at all (a position difference, which near-match judges).
  const near = (src, dst, x, y) => {
    const p = y * w + x;
    let partners = 0;
    for (let dy = -1; dy <= 1; dy++) {
      const yy = y + dy;
      if (yy < 0 || yy >= h) continue;
      for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx;
        if (xx < 0 || xx >= w) continue;
        const q = yy * w + xx;
        if (dst.depth[q] < HUE_MIN_DEPTH / 2) continue;
        partners++;
        const dot = src.v[p * 3] * dst.v[q * 3] + src.v[p * 3 + 1] * dst.v[q * 3 + 1] + src.v[p * 3 + 2] * dst.v[q * 3 + 2];
        if (dot >= cos) return 1;
      }
    }
    return partners ? 0 : -1;
  };
  const checked = new Uint8Array(w * h);
  const bad = new Uint8Array(w * h);
  let n = 0;
  let m = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const p = y * w + x;
      const ra = A.depth[p] >= HUE_MIN_DEPTH ? near(A, B, x, y) : -1;
      const rb = B.depth[p] >= HUE_MIN_DEPTH ? near(B, A, x, y) : -1;
      if (ra === -1 && rb === -1) continue;
      checked[p] = 1;
      n++;
      if (ra === 0 || rb === 0) {
        bad[p] = 1;
        m++;
      }
    }
  }
  const { worst, worstAt } = worstWindow(checked, bad, w, h, HUE_MIN_PIXELS);
  return { mismatch: bad, hue_pixels: n, hue_mismatch_pixels: m, hue_worst_window: worst, hue_worst_window_at: worstAt };
}

/**
 * Near-match diff of two same-sized images. Returns the mismatch map, the
 * global share of mismatched ink and the worst window's share.
 */
export function nearMatch(a, b, { radius = NEAR_RADIUS, tolerance = NEAR_TOLERANCE } = {}) {
  const { width: w, height: h } = a;
  const A = a.data;
  const B = b.data;
  const isInk = (d, i) => d[i] < INK_LEVEL || d[i + 1] < INK_LEVEL || d[i + 2] < INK_LEVEL;
  const near = (src, dst, x, y) => nearPixel(src, dst, w, h, x, y, radius, tolerance);
  const ink = new Uint8Array(w * h);
  const bad = new Uint8Array(w * h);
  let inkN = 0;
  let badN = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      if (!isInk(A, i) && !isInk(B, i)) continue;
      ink[y * w + x] = 1;
      inkN++;
      if (!near(A, B, x, y) || !near(B, A, x, y)) {
        bad[y * w + x] = 1;
        badN++;
      }
    }
  }
  const { worst, worstAt } = worstWindow(ink, bad, w, h, WINDOW_MIN_INK);
  return { mismatch: bad, near_mismatch_pixels: badN, near_ink_share: inkN ? badN / inkN : 0, worst_window: worst, worst_window_at: worstAt };
}

/** True when both images have the same size and identical RGBA bytes. */
export function pixelsIdentical(a, b) {
  return a.width === b.width && a.height === b.height && Buffer.compare(a.data, b.data) === 0;
}

/**
 * Perceptual diff of `expected` (Chrome) against `actual`.
 * Returns the score object and a diff image (red: near-mismatch, magenta: wrong
 * colour on a solid area, orange: pixelmatch only, yellow: anti-aliasing only).
 */
export function diff(expected, actual) {
  const width = Math.max(expected.width, actual.width);
  const height = Math.max(expected.height, actual.height);
  const a = padTo(expected, width, height);
  const b = padTo(actual, width, height);
  const out = new PNG({ width, height });
  const diffPixels = pixelmatch(a.data, b.data, out.data, width, height, {
    threshold: PIXEL_THRESHOLD,
    includeAA: false,
    alpha: 0.15,
    diffColor: [255, 170, 0],
  });
  const ink = countInkUnion(a, b);
  const near = nearMatch(a, b);
  const colour = colourMatch(a, b);
  const hue = hueMatch(a, b);
  // Near-match mismatches in solid red, colour mismatches in magenta, over pixelmatch's faded diff.
  for (let p = 0; p < near.mismatch.length; p++) {
    if (near.mismatch[p]) out.data.set([255, 0, 0, 255], p * 4);
    else if (colour.mismatch[p] || hue.mismatch[p]) out.data.set([230, 0, 230, 255], p * 4);
  }
  return {
    score: {
      // Primary score: worst 32x32 window's share of near-mismatched ink.
      worst_window: near.worst_window,
      worst_window_at: near.worst_window_at,
      near_mismatch_pixels: near.near_mismatch_pixels,
      near_ink_share: near.near_ink_share,
      // Colour check on solid areas (tolerance COLOUR_TOLERANCE per channel).
      colour_worst_window: colour.colour_worst_window,
      colour_worst_window_at: colour.colour_worst_window_at,
      colour_mismatch_pixels: colour.colour_mismatch_pixels,
      solid_pixels: colour.solid_pixels,
      // Hue check on strongly inked pixels (thin strokes and text), HUE_TOLERANCE_DEG.
      hue_worst_window: hue.hue_worst_window,
      hue_worst_window_at: hue.hue_worst_window_at,
      hue_mismatch_pixels: hue.hue_mismatch_pixels,
      hue_pixels: hue.hue_pixels,
      width,
      height,
      size_match: expected.width === actual.width && expected.height === actual.height,
      expected_size: [expected.width, expected.height],
      actual_size: [actual.width, actual.height],
      diff_pixels: diffPixels,
      ink_pixels: ink,
      expected_ink_pixels: countInk(expected),
      actual_ink_pixels: countInk(actual),
      // pixelmatch (secondary): share of the inked area that differs, AA ignored.
      diff_ratio_ink: ink === 0 ? 0 : diffPixels / ink,
      // Share of the whole page that differs (diluted by white space; for reference).
      diff_ratio_page: diffPixels / (width * height),
    },
    image: out,
  };
}

/** Lays images out left to right on white with a gap and a grey frame. */
export function sideBySide(images) {
  const height = Math.max(...images.map((i) => i.height)) + 2 * GAP;
  const width = images.reduce((w, i) => w + i.width + GAP, GAP);
  const out = new PNG({ width, height });
  for (let i = 0; i < out.data.length; i += 4) {
    out.data[i] = out.data[i + 1] = out.data[i + 2] = 200;
    out.data[i + 3] = 255;
  }
  let x = GAP;
  for (const img of images) {
    PNG.bitblt(img, out, 0, 0, img.width, img.height, x, GAP);
    x += img.width + GAP;
  }
  return out;
}
