// Unbounded recursion, not caught.
function f(n) {
  return f(n + 1) + 1;
}
f(0);
