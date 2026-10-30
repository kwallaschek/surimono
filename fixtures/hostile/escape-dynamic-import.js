// Dynamic import of host modules, files, URLs and data: URLs must all fail.
// (A SyntaxError because import() is disabled is also acceptable.)
const specs = ["os", "std", "fs", "node:fs", "qjs:os", "/etc/passwd", "./x.js", "https://example.com/x.js", "data:text/javascript,export default 1"];
Promise.allSettled(specs.map((s) => import(s))).then((rs) =>
  rs.every((r) => r.status === "rejected") ? "denied" : "ESCAPED: " + specs.filter((_, i) => rs[i].status === "fulfilled").join(","),
);
