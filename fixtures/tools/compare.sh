#!/usr/bin/env bash
# Perceptual diff of a surimono + fulgur PDF against a fixture's Chrome reference.
#
#   fixtures/tools/compare.sh [--label NAME] [--report-only] <fixture-name-or-dir> <pdf> <outdir>
#
# Writes <outdir>/score.json and <outdir>/side-by-side-<page>.png
# (Chrome | <label> | diff). Exits 1 if a page fails the threshold.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
if [ ! -d "$here/node_modules" ]; then (cd "$here" && npm ci --no-audit --no-fund); fi
command -v pdftoppm >/dev/null || { echo "pdftoppm not found: install poppler-utils" >&2; exit 1; }
exec node "$here/compare.mjs" "$@"
