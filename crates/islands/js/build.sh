#!/usr/bin/env bash
# Builds the built-in renderer bundles that surimono-islands compiles in.
#
#   ./build.sh           install the pinned packages (npm ci), build every
#                        bundle into ../bundles/, copy the licences and
#                        rewrite ../bundles/SHA256SUMS
#   ./build.sh --check   build into a temporary directory and fail unless the
#                        result is byte-identical to the committed bundles and
#                        matches SHA256SUMS (proves the bundles are reproducible)
#
# The bundles are committed, so `cargo build` never needs Node or npm. The
# exact versions are pinned in package.json and package-lock.json, and esbuild
# output is deterministic, so a rebuild gives the same bytes.
set -euo pipefail
cd "$(dirname "$0")"

check=0
case "${1:-}" in
  --check) check=1 ;;
  "") ;;
  *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

npm ci --no-audit --no-fund >/dev/null

out=../bundles
if [ "$check" = 1 ]; then
  out="$(mktemp -d)"
fi

version() { node -p "require('./node_modules/$1/package.json').version"; }

# esbuild: one classic script (IIFE) per renderer that sets the global
# `entry(specJson, ctx)`. ES2020 syntax, which QuickJS-ng runs natively;
# production mode strips the libraries' development checks and their
# references to `process`.
meta="$(mktemp -d)"
trap 'rm -rf "$meta"; [ "$check" = 1 ] && rm -rf "$out"' EXIT
bundle() {
  local name="$1" src="$2" banner="$3"; shift 3
  mkdir -p "$out/$name"
  node_modules/.bin/esbuild "$src" \
    --bundle --format=iife --platform=browser --target=es2020 --minify \
    --charset=ascii --legal-comments=eof \
    --define:process.env.NODE_ENV='"production"' \
    --banner:js="$banner" \
    --metafile="$meta/$name.json" \
    --log-level=warning \
    "$@" \
    --outfile="$out/$name/$name.min.js"
}

# --- ECharts ------------------------------------------------------------------
echarts_v="$(version echarts)"
zrender_v="$(version zrender)"
tslib_v="$(version tslib)"
bundle echarts src/echarts.js \
  "/* surimono echarts renderer: Apache ECharts $echarts_v (Apache-2.0), zrender $zrender_v (BSD-3-Clause), tslib $tslib_v (0BSD). Licences and NOTICE in this directory. */"
cp node_modules/echarts/LICENSE "$out/echarts/LICENSE"
cp node_modules/echarts/NOTICE "$out/echarts/NOTICE"
cp node_modules/echarts/licenses/LICENSE-d3 "$out/echarts/LICENSE-d3"
cp node_modules/zrender/LICENSE "$out/echarts/LICENSE-zrender"
cp node_modules/tslib/LICENSE.txt "$out/echarts/LICENSE-tslib"

# --- Vega and Vega-Lite (one bundle, two entries) ------------------------------
# vega-canvas is replaced by src/vega-canvas.js, which measures text through
# the host and has no raster canvas.
vega_v="$(version vega)"
vegalite_v="$(version vega-lite)"
bundle vega src/vega.js \
  "/* surimono vega and vega-lite renderers: Vega $vega_v and Vega-Lite $vegalite_v (BSD-3-Clause) with d3 and other dependencies. Licences in THIRD_PARTY_LICENSES.txt in this directory. */" \
  --alias:vega-canvas=./src/vega-canvas.js
cp node_modules/vega/LICENSE "$out/vega/LICENSE"
node tools/licenses.js "$meta/vega.json" vega > "$out/vega/THIRD_PARTY_LICENSES.txt"

# --- Checksums ------------------------------------------------------------------
sums() { (cd "$1" && find . -type f ! -name SHA256SUMS | LC_ALL=C sort | sed 's|^\./||' | xargs sha256sum); }

if [ "$check" = 1 ]; then
  if ! diff <(sums "$out") ../bundles/SHA256SUMS; then
    echo "build.sh --check: the rebuilt bundles differ from ../bundles/SHA256SUMS" >&2
    exit 1
  fi
  (cd ../bundles && sha256sum --quiet -c SHA256SUMS)
  echo "build.sh --check: bundles are reproducible and match SHA256SUMS"
else
  sums "$out" > "$out/SHA256SUMS"
  cat "$out/SHA256SUMS"
fi
