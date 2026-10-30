// join over a Proxy that reports a 2**40 length: every index read calls the
// get trap, so the loop alternates between native code and JavaScript.
const p = new Proxy({}, { get: (_target, key) => (key === "length" ? 2 ** 40 : undefined) });
Array.prototype.join.call(p, "");
"finished";
