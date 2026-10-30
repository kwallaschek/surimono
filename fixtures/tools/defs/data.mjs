// Deterministic synthetic data for fixtures. No Math.random, no clock.

/** mulberry32: a small seeded PRNG; same seed, same sequence. */
export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const round = (x, digits = 0) => {
  const f = 10 ** digits;
  return Math.round(x * f) / f;
};

/** ISO dates (UTC) for `days` consecutive days from `start` (YYYY-MM-DD). */
export function isoDays(start, days) {
  const t0 = Date.parse(`${start}T00:00:00Z`);
  return Array.from({ length: days }, (_, i) => new Date(t0 + i * 86400000).toISOString().slice(0, 10));
}

/** A random walk around `base` with drift and noise, rounded. */
export function walk(next, n, base, drift, noise, digits = 0) {
  let v = base;
  return Array.from({ length: n }, () => {
    v = Math.max(0, v + drift + (next() - 0.5) * 2 * noise);
    return round(v, digits);
  });
}

export const fmtInt = (n) => Math.round(n).toLocaleString("en-US");
export const fmtYen = (n) => `¥${Math.round(n).toLocaleString("ja-JP")}`;
