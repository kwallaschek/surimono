// Keeps one Chrome running for Grover's warm mode (browser_ws_endpoint), the way
// a Grover deployment with a long-lived browser works. Launches Chrome with the
// options Grover's own processor.cjs uses (temporary profile, no sandbox), prints
// {"ws": "<endpoint>"} on one line, and closes Chrome when stdin closes.
//
//   node bench/grover/browser.cjs <chrome-executable>

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const puppeteer = require("puppeteer");

(async () => {
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "grover-bench-"));
  const browser = await puppeteer.launch({
    executablePath: process.argv[2],
    args: ["--no-sandbox", "--disable-setuid-sandbox"],
    userDataDir,
  });
  process.stdout.write(`${JSON.stringify({ ws: browser.wsEndpoint() })}\n`);
  process.stdin.resume();
  process.stdin.on("end", async () => {
    await browser.close();
    fs.rmSync(userDataDir, { recursive: true, force: true });
    process.exit(0);
  });
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
