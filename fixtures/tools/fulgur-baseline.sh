#!/usr/bin/env bash
# The fulgur baseline: fulgur printing Chrome's own island SVGs, compared with
# the Chrome references (see baseline.mjs).
#
#   fixtures/tools/fulgur-baseline.sh <outdir> [fixture ...]
#
# Needs fulgur 0.40.0 on PATH or in $FULGUR
# (cargo install fulgur-cli --version =0.40.0 --locked).
# Writes <outdir>/baseline.md and baseline.json, and per fixture
# <outdir>/<name>/fulgur/{page,island-<i>}/side-by-side-*.png.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:?usage: fulgur-baseline.sh <outdir> [fixture ...]}"; shift
if [ ! -d "$here/node_modules" ]; then (cd "$here" && npm ci --no-audit --no-fund); fi
# Chrome renders and prepass.html (check mode never touches committed references).
node "$here/reference.mjs" --out "$out" "$@" || echo "note: reference check reported differences; the baseline still runs"
node "$here/baseline.mjs" --out "$out" "$@"
