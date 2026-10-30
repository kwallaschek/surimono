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

/**
 * Near-match diff of two same-sized images. Returns the mismatch map, the
 * global share of mismatched ink and the worst window's share.
 */
export function nearMatch(a, b, { radius = NEAR_RADIUS, tolerance = NEAR_TOLERANCE } = {}) {
  const NEAR_RADIUS_ = radius;
  const NEAR_TOLERANCE_ = tolerance;
  const { width: w, height: h } = a;
  const A = a.data;
  const B = b.data;
  const isInk = (d, i) => d[i] < INK_LEVEL || d[i + 1] < INK_LEVEL || d[i + 2] < INK_LEVEL;
  const near = (src, dst, x, y) => {
    const i = (y * w + x) * 4;
    for (let dy = -NEAR_RADIUS_; dy <= NEAR_RADIUS_; dy++) {
      const yy = y + dy;
      if (yy < 0 || yy >= h) continue;
      for (let dx = -NEAR_RADIUS_; dx <= NEAR_RADIUS_; dx++) {
        const xx = x + dx;
        if (xx < 0 || xx >= w) continue;
        const j = (yy * w + xx) * 4;
        if (Math.abs(src[i] - dst[j]) <= NEAR_TOLERANCE_ && Math.abs(src[i + 1] - dst[j + 1]) <= NEAR_TOLERANCE_ && Math.abs(src[i + 2] - dst[j + 2]) <= NEAR_TOLERANCE_) return true;
      }
    }
    return false;
  };
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
  let worst = 0;
  let worstAt = null;
  const step = WINDOW / 2;
  for (let y0 = 0; y0 < h; y0 += step) {
    for (let x0 = 0; x0 < w; x0 += step) {
      let n = 0;
      let m = 0;
      for (let y = y0; y < Math.min(h, y0 + WINDOW); y++) {
        for (let x = x0; x < Math.min(w, x0 + WINDOW); x++) {
          n += ink[y * w + x];
          m += bad[y * w + x];
        }
      }
      if (n >= WINDOW_MIN_INK && m / n > worst) {
        worst = m / n;
        worstAt = [x0, y0];
      }
    }
  }
  return { mismatch: bad, near_mismatch_pixels: badN, near_ink_share: inkN ? badN / inkN : 0, worst_window: worst, worst_window_at: worstAt };
}

/** True when both images have the same size and identical RGBA bytes. */
export function pixelsIdentical(a, b) {
  return a.width === b.width && a.height === b.height && Buffer.compare(a.data, b.data) === 0;
}

/**
 * Perceptual diff of `expected` (Chrome) against `actual`.
 * Returns the score object and a diff image (red: differs, yellow: anti-aliasing only).
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
  // Near-match mismatches drawn in solid red over pixelmatch's faded diff.
  for (let p = 0; p < near.mismatch.length; p++) {
    if (near.mismatch[p]) out.data.set([255, 0, 0, 255], p * 4);
  }
  return {
    score: {
      // Primary score: worst 32x32 window's share of near-mismatched ink.
      worst_window: near.worst_window,
      worst_window_at: near.worst_window_at,
      near_mismatch_pixels: near.near_mismatch_pixels,
      near_ink_share: near.near_ink_share,
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
