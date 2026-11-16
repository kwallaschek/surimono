#!/usr/bin/env node
// The Chromium defaults surimono's printer-safe output relies on, asked of the
// pinned Chromium. Run by reference.sh, where the references' Chromium runs.
//
//   node chrome-defaults.mjs [--json FILE]
//
// Printer-safe output adds `:root{text-autospace:no-autospace}` to pages with
// Japanese text (Q-PKG-04, DECISIONS 2026-11-16). That is neutral in Chromium only
// while Chromium's computed initial value of `text-autospace` is `no-autospace`
// (CSS Text 4 says `normal`; Chromium 153 computes `no-autospace`). A Chromium pin
// bump that changes it fails here, with what to do, before any reference moves.
// Exits 0 when every default is as expected, 1 when one isn't, 2 when Chromium
// can't be asked.

import fs from "node:fs";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";

/** What printer-safe output takes for granted, and where it says so. */
export const EXPECTED = [
  {
    name: "text-autospace, initial value",
    property: "text-autospace",
    expected: "no-autospace",
    why: "printer-safe output adds `:root{text-autospace:no-autospace}` to pages with Japanese text, which is neutral in Chromium only while that is Chromium's initial value (DECISIONS 2026-11-16, Q-PKG-04)",
  },
];

/**
 * The page asked: the root with no author value, an element set to `initial`,
 * and one with Japanese and Latin text, under print media as Chromium prints.
 */
const PAGE = `<!doctype html><html lang="ja"><head><meta charset="utf-8">
<style>#i{text-autospace:initial}</style></head>
<body><p id="p">2026年の売上</p><p id="i">ABC漢字</p></body></html>`;

/**
 * Judges what Chromium answered: `observed` maps each place asked (`root`,
 * `initial`, `text`) to its computed value. Returns the failures, each a
 * sentence that says what moved and what to do.
 */
export function judge(observed, chromium = "this Chromium") {
  const failures = [];
  for (const want of EXPECTED) {
    for (const [where, got] of Object.entries(observed[want.property] ?? {})) {
      if (got !== want.expected) {
        failures.push(
          `${chromium} computes ${want.property} as ${JSON.stringify(got)} on ${where}, not ${JSON.stringify(want.expected)}. ${want.why[0].toUpperCase()}${want.why.slice(1)}. ` +
            "Before this Chromium pin lands: measure the rule again on the island and real-world pages (Chromium and the printers), and change or drop it in crates/surimono/src/print/safe/ (the packaging engineer's), or keep the old pin.",
        );
      }
    }
    if (!Object.keys(observed[want.property] ?? {}).length) failures.push(
        `${chromium} gave no value for ${want.property}: it may not support the property any more. ${want.why[0].toUpperCase()}${want.why.slice(1)}. ` +
          `Before this Chromium pin lands: check in it that the property is gone (getComputedStyle answers "" and CSS.supports("${want.property}: ${want.expected}") is false), then measure the island and real-world pages with and without the rule (Chromium and the printers) and change or drop it in crates/surimono/src/print/safe/ (the packaging engineer's), or keep the old pin.`,
      );
  }
  return failures;
}

async function main() {
  const { values } = parseArgs({ options: { json: { type: "string" } } });
  const { launchChromium } = await import("./lib/fixtures.mjs");
  let browser;
  try {
    browser = await launchChromium();
  } catch (e) {
    console.error(`chrome-defaults: can't start the pinned Chromium: ${e.message.split("\n")[0]}`);
    process.exit(2);
  }
  const chromium = `Chromium ${browser.version()}`;
  const page = await browser.newPage();
  await page.setContent(PAGE);
  await page.emulateMedia({ media: "print" });
  const observed = {};
  for (const want of EXPECTED) {
    observed[want.property] = await page.evaluate((prop) => {
      const value = (el) => getComputedStyle(el).getPropertyValue(prop);
      return { root: value(document.documentElement), initial: value(document.getElementById("i")), text: value(document.getElementById("p")) };
    }, want.property);
  }
  await browser.close();
  const failures = judge(observed, chromium);
  if (values.json) fs.writeFileSync(values.json, `${JSON.stringify({ chromium, observed, failures }, null, 2)}\n`);
  for (const want of EXPECTED) console.log(`${chromium}: ${want.property} = ${JSON.stringify(observed[want.property])} (expected ${want.expected})`);
  if (failures.length) {
    for (const f of failures) console.error(`FAIL: ${f}`);
    process.exit(1);
  }
  console.log("chrome-defaults: every default printer-safe output relies on is as expected");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) await main();
