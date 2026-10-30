#!/usr/bin/env bash
# Offline checks of the committed fixtures (run by scripts/ci.sh on every PR):
#   fixtures/fonts/verify.py          test fonts match their pinned size and sha256
#   fixtures/hostile/verify.py        the hostile-script manifest is well-formed
#   fixtures/tools/gen-islands.mjs    the island fixtures are what the generator writes
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
echo "fixture verifiers run: $ran"
