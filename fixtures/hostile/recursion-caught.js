// Unbounded recursion that the script catches, as browsers allow (feature
// detection code does this). The sandbox must survive and stay usable.
function f(n) {
  return f(n + 1) + 1;
}
let r = "not caught";
try {
  f(0);
} catch (e) {
  r = "caught";
}
// The runtime still works after unwinding.
r + ":" + [1, 2, 3].map((x) => x * 2).join(",");
