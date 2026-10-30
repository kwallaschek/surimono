// Node, CommonJS and QuickJS host globals must not exist.
const names = ["require", "module", "exports", "process", "Buffer", "global", "Deno", "Bun", "std", "os", "scriptArgs", "importScripts"];
names.filter((n) => typeof globalThis[n] !== "undefined").join(",") || "none";
