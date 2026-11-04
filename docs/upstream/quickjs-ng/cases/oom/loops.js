function f(o, a) {
  for (const x of a) { o.x = x; }
  for (const k in o) { o.k = k; }
  for (const x of a) { o.x = x; }
  for (const k in o) { o.k = k; }
  for (const x of a) { o.x = x; }
  for (const k in o) { o.k = k; }
  for (const x of a) { o.x = x; }
  for (const k in o) { o.k = k; }
  for (const x of a) { o.x = x; }
  for (const k in o) { o.k = k; }
}
function g(o) {
  const { a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = 7, h = 8, i = 9, j = 10 } = o;
  const [k = 1, l = 2, m = 3, n = 4, p = 5, q = 6, r = 7, s = 8] = o.t;
  return a + b + c + d + e + f + g + h + i + j + k + l + m + n + p + q + r + s;
}
