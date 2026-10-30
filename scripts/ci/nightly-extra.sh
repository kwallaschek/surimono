#!/usr/bin/env bash
# Extra nightly checks on Linux, so new checks need no workflow change. These
# need npm or the network, so they stay out of the PR gate (scripts/ci.sh).
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

ran=0
# The fixture tooling's JavaScript tests need its npm dependencies.
if ls fixtures/tools/test/*.test.mjs >/dev/null 2>&1; then
  step "fixtures: tool tests (node, after npm ci)"
  (cd fixtures/tools && npm ci --no-audit --no-fund)
  node --test fixtures/tools/test/*.test.mjs; ran=$((ran + 1))
fi
# The real-world suite checks its pinned packages against registry.npmjs.org.
if [ -f fixtures/real-world/verify.py ]; then
  step "fixtures: real-world suite (needs registry.npmjs.org)"
  python3 -I fixtures/real-world/verify.py; ran=$((ran + 1))
fi
echo "nightly extra checks run: $ran"
