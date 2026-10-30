// Walks every property reachable from globalThis (three levels deep) looking
// for host capabilities. At the top level: any Node, Deno, Bun, QuickJS-libc
// or network global. At any depth: names of file, process or script-loading
// functions (std.loadFile, os.exec and friends). The report must be empty.
const topLevel = /^(require|process|module|exports|Buffer|global|std|os|scriptArgs|fetch|XMLHttpRequest|WebSocket|EventSource|Worker|SharedWorker|importScripts|Deno|Bun)$/;
const anyLevel = /^(readFile|readFileSync|writeFile|writeFileSync|loadFile|evalScript|loadScript|popen|spawn|getenv|setenv|urlGet|readdir|realpath|chdir|kill|exec)$/;
const found = [];
const seen = new Set();
function walk(obj, path, depth) {
  if (obj === null || (typeof obj !== "object" && typeof obj !== "function") || seen.has(obj) || depth > 3) return;
  seen.add(obj);
  let keys = [];
  try {
    keys = Reflect.ownKeys(obj);
  } catch (e) {
    return;
  }
  for (const k of keys) {
    if (typeof k !== "string") continue;
    // RegExp.prototype.exec is the one standard function with a listed name.
    if ((depth === 0 && topLevel.test(k)) || (anyLevel.test(k) && !(k === "exec" && obj === RegExp.prototype))) found.push(path + "." + k);
    let v;
    try {
      v = obj[k];
    } catch (e) {
      continue;
    }
    walk(v, path + "." + k, depth + 1);
  }
}
walk(globalThis, "globalThis", 0);
found.length ? found.sort().join(",") : "clean";
