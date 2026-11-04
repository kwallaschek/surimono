// Preload for scripts that call playwright.chromium.launch() with no channel
// (crates/gfx/tests/chrome/*.mjs): `node --require` this file and their launch
// uses the full Chromium in new headless mode, the same binary as the references
// (lib/fixtures.mjs, launchChromium). Without it Playwright looks for the separate
// headless shell, which CI doesn't install (reference.sh, --no-shell) and which
// would be a different browser from the one the references come from.
"use strict";

/** Launch options with the references' channel unless the caller named a browser. */
function pinnedOptions(options = {}) {
  if (options.channel || options.executablePath) return options;
  return { ...options, channel: "chromium" };
}

module.exports = { pinnedOptions };

const { chromium } = require("playwright");
const launch = chromium.launch;
chromium.launch = function (options) {
  return launch.call(this, pinnedOptions(options));
};
