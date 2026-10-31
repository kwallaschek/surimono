// Dynamic import of host modules, files, URLs and data: URLs must all fail.
// (A SyntaxError because import() is disabled is also acceptable.)
//
// modules/escape-target.mjs is a real module, named the ways a loader could
// resolve it: from the repository root, from a crate directory (cargo test's
// working directory) and relative to this script (manifest.toml's header says
// how the harness names scripts). A loader that reads files finds one of them.
// A SyntaxError for /etc/passwd also means the file was read: it was parsed.
const target = "fixtures/hostile/modules/escape-target.mjs";
const specs = [
  "os", "std", "fs", "node:fs", "qjs:os",
  "/etc/passwd", "./x.js",
  target, `./${target}`, `../../${target}`, "./modules/escape-target.mjs", "modules/escape-target.mjs",
  "https://example.com/x.js", "data:text/javascript,export default 1",
];
Promise.allSettled(specs.map((s) => import(s))).then((rs) => {
  const escaped = specs.filter((s, i) =>
    rs[i].status === "fulfilled" || (s === "/etc/passwd" && rs[i].reason instanceof SyntaxError));
  return escaped.length ? "ESCAPED: " + escaped.join(",") : "denied";
});
