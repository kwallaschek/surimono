#!/usr/bin/env bash
# Chrome reference renders for the visual fixtures.
#
#   fixtures/tools/reference.sh [--update] <outdir> [fixture ...]
#
# Check mode (default, and what CI runs): render every fixture in the pinned
# Chromium, rasterise at 96 dpi, and compare with the committed
# fixtures/<kind>/<name>/reference/chrome-<page>.png. Exits 1 if a page changed
# or a reference is missing. Results: <outdir>/reference-report.json, and for
# each fixture <outdir>/<name>/ (chrome.pdf, chrome-N.png, prepass.html,
# render.json, check-N.png side-by-side for changed pages).
#
# --update rewrites the committed references. Use it only for a new fixture or a
# deliberate Chromium/library upgrade, and say so in the pull request.
#
# On a stock CI runner ($CI set) it installs what it needs first: npm ci, the
# Chromium build that matches the pinned Playwright, and poppler-utils.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"

args=()
if [ "${1:-}" = "--update" ]; then args+=(--update); shift; fi
out="${1:?usage: reference.sh [--update] <outdir> [fixture ...]}"; shift
mkdir -p "$out"
out="$(cd "$out" && pwd)"

if [ -n "${CI:-}" ]; then
  echo "==> CI: installing pinned fixture tools"
  (cd "$here" && npm ci --no-audit --no-fund)
  # Full Chromium only (the harness uses channel "chromium", not the headless shell).
  (cd "$here" && npx --no-install playwright install --with-deps --no-shell chromium)
  if ! command -v pdftoppm >/dev/null; then
    sudo apt-get update -qq && sudo apt-get install -y -qq poppler-utils
  fi
elif [ ! -d "$here/node_modules" ]; then
  (cd "$here" && npm ci --no-audit --no-fund)
fi

command -v pdftoppm >/dev/null || { echo "pdftoppm not found: install poppler-utils" >&2; exit 1; }

echo "==> fixture inputs match their generator"
node "$here/gen-islands.mjs" --check

echo "==> Chrome references"
node "$here/reference.mjs" --out "$out" "${args[@]}" "$@"
