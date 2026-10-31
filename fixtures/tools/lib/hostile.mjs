// Reads fixtures/hostile/manifest.toml for the Node tools. A minimal reader
// of the few keys they need (id, file, and the files of `steps`), checked
// against Python's tomllib by test/hostile.test.mjs.

import fs from "node:fs";
import path from "node:path";
import { FIXTURES_DIR } from "./fixtures.mjs";

export const HOSTILE_DIR = path.join(FIXTURES_DIR, "hostile");

/** The cases of a manifest's text as [{ id, step, file }], one entry per step of a `steps` case (step 1, 2, ...; null otherwise). */
export function hostileCases(text = fs.readFileSync(path.join(HOSTILE_DIR, "manifest.toml"), "utf8")) {
  const out = [];
  for (const block of text.split(/^\[\[case\]\]\s*$/m).slice(1)) {
    const id = /^id\s*=\s*"([^"]+)"/m.exec(block)?.[1];
    if (!id) throw new Error(`a [[case]] without an id: ${block.slice(0, 80)}`);
    const file = /^file\s*=\s*"([^"]+)"/m.exec(block);
    if (file) {
      out.push({ id, step: null, file: file[1] });
      continue;
    }
    const steps = /^steps\s*=\s*\[([\s\S]*?)\]\s*$/m.exec(block);
    const files = [...(steps?.[1] ?? "").matchAll(/file\s*=\s*"([^"]+)"/g)].map((m) => m[1]);
    if (!files.length) throw new Error(`case ${id} has neither file nor steps`);
    files.forEach((f, i) => out.push({ id, step: i + 1, file: f }));
  }
  return out;
}
