function f(a, b) {
  let x = a + b, y = a - b, z = [a, b, x, y];
  for (let i = 0; i < 3; i++) { x += i; y -= i; z.push(x * y); }
  if (x > y) { z.push(`x${x}`); } else { z.push(`y${y}`); }
  try { z.push(JSON.stringify({ a, b, x, y })); } finally { x = 0; }
  return z.join();
}
f(1, 2);
