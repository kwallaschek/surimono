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
status=0
node "$here/reference.mjs" --out "$out" "${args[@]}" "$@" || status=$?

# gfx's own Chrome checks (crates/gfx/tests/chrome/, owned by the graphics
# engineer; run from here so they use the same pinned Chromium):
#   svg_inline.mjs   sanitized island SVG stays inert when inlined in an HTML page
#   svg_outline.mjs  outlined island text looks like Chrome's own text in the same fonts
# Their inputs come from `cargo test -p surimono-gfx --test svg`. Skipped, with a
# note, in --update mode, for a subset of fixtures, without cargo, or with
# SURIMONO_SKIP_GFX_CHROME=1. Logs: <out>/gfx-svg-inline.log, <out>/gfx-svg-outline.log.
repo="$(cd "$here/../.." && pwd)"
if [ ${#args[@]} -gt 0 ] || [ $# -gt 0 ] || [ -n "${SURIMONO_SKIP_GFX_CHROME:-}" ]; then
  echo "==> gfx Chrome checks: skipped (--update, a fixture subset, or SURIMONO_SKIP_GFX_CHROME)"
elif ! command -v cargo >/dev/null; then
  echo "==> gfx Chrome checks: skipped, cargo not found" >&2
else
  echo "==> gfx Chrome checks: inputs (cargo test -p surimono-gfx --test svg)"
  target="$(cd "$repo" && cargo metadata --format-version 1 --no-deps |
    node -e 'let s="";process.stdin.on("data",(d)=>(s+=d)).on("end",()=>console.log(JSON.parse(s).target_directory))')"
  if (cd "$repo" && cargo test -p surimono-gfx --test svg -- --exact hostile_svg_is_neutralised real_charts_print_as_outlines); then
    # The scripts load playwright with require(); resolve it to the pinned one here.
    export NODE_PATH="$here/node_modules${NODE_PATH:+:$NODE_PATH}"
    echo "==> gfx Chrome checks: svg_inline.mjs"
    node "$repo/crates/gfx/tests/chrome/svg_inline.mjs" "$target/tmp/hostile-svg.json" 2>&1 | tee "$out/gfx-svg-inline.log"
    [ "${PIPESTATUS[0]}" -eq 0 ] || status=1
    echo "==> gfx Chrome checks: svg_outline.mjs"
    node "$repo/crates/gfx/tests/chrome/svg_outline.mjs" "$target/tmp/outlined" "$repo/fixtures/fonts" 2>&1 | tee "$out/gfx-svg-outline.log"
    [ "${PIPESTATUS[0]}" -eq 0 ] || status=1
  else
    echo "gfx Chrome checks: cargo test -p surimono-gfx --test svg failed" >&2
    status=1
  fi
fi
exit "$status"
