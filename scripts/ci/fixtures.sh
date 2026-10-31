#!/usr/bin/env bash
# Offline checks of the committed fixtures (run by scripts/ci.sh on every PR):
#   fixtures/fonts/verify.py          test fonts match their pinned size and sha256
#   fixtures/hostile/verify.py        the hostile-script manifest is well-formed
#   fixtures/tools/gen-islands.mjs    the island fixtures are what the generator writes
#   bench/report/gen-report.mjs       the committed benchmark report is what the generator writes
#   fixtures/tools/test/test_*.py     unit tests of the fixture tooling
#   fixtures/real-world/verify.py     the embedded library builds match the manifest (--embedded-only)
#   fixtures/real-world/test_*.py     unit tests of the real-world suite's tooling
#   bench/tools/test_*.py             unit tests of the benchmark tooling
# docs/COMPAT.md is regenerated from a CI run's published results, so it isn't
# checked here (compat.py --check without results rebuilds an all-"not run"
# file). The checks that need npm or the network run nightly instead
# (nightly-extra.sh):
# the node tests of the fixture tooling need its npm dependencies (pngjs).
# None needs the network or npm. Each runs only once its file exists, so this
# works before and after the conformance branches land.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

ran=0
if [ -f fixtures/fonts/verify.py ]; then
  step "fixtures: fonts"; python3 -I fixtures/fonts/verify.py; ran=$((ran + 1))
fi
if [ -f fixtures/hostile/verify.py ]; then
  step "fixtures: hostile scripts"; python3 -I fixtures/hostile/verify.py; ran=$((ran + 1))
fi
if [ -f fixtures/tools/gen-islands.mjs ]; then
  command -v node >/dev/null || { echo "node (18 or later) is needed for fixtures/tools/gen-islands.mjs" >&2; exit 1; }
  step "fixtures: island inputs (gen-islands --check)"; node fixtures/tools/gen-islands.mjs --check; ran=$((ran + 1))
fi
if [ -f bench/report/gen-report.mjs ]; then
  command -v node >/dev/null || { echo "node (18 or later) is needed for bench/report/gen-report.mjs" >&2; exit 1; }
  step "bench: report (gen-report --check)"; node bench/report/gen-report.mjs --check; ran=$((ran + 1))
fi
if [ -d fixtures/tools/test ] && ls fixtures/tools/test/test_*.py >/dev/null 2>&1; then
  step "fixtures: tool tests (python)"
  python3 -I -m unittest discover -s fixtures/tools/test -p 'test_*.py'; ran=$((ran + 1))
fi
if [ -f fixtures/real-world/verify.py ]; then
  step "fixtures: real-world embedded builds (offline)"
  python3 -I fixtures/real-world/verify.py --embedded-only; ran=$((ran + 1))
fi
if ls fixtures/real-world/test_*.py >/dev/null 2>&1; then
  step "fixtures: real-world tool tests (python)"
  python3 -I -m unittest discover -s fixtures/real-world -p 'test_*.py'; ran=$((ran + 1))
fi
if ls bench/tools/test_*.py >/dev/null 2>&1; then
  step "bench: tool tests (python)"
  python3 -I -m unittest discover -s bench/tools -p 'test_*.py'; ran=$((ran + 1))
fi
echo "fixture verifiers run: $ran"
