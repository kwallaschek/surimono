#!/usr/bin/env bash
# Runs the vendored WPT slices through wpt/runner and writes wpt-results.json to
# $CI_JOB_OUT, which publishes it to the ci-results branch. The runner exits 0
# whatever the tests report, so this fails only if it can't build or run; the
# rates are scored elsewhere (compat.py --wpt), never here. nightly-extra.sh runs
# it once wpt/runner exists.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

out="${CI_JOB_OUT:-$REPO_ROOT/ci-out/nightly-extra}"
mkdir -p "$out"
# A standalone workspace with its own Cargo.lock (wpt/README.md, "Runner").
cargo build --manifest-path wpt/runner/Cargo.toml --locked
"${CARGO_TARGET_DIR:-wpt/runner/target}/debug/surimono-wpt-runner" \
  --root wpt --out "$out/wpt-results.json" --commit "$(git rev-parse HEAD)"
echo "wrote $out/wpt-results.json"
