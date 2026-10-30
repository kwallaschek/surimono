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
# The fulgur print test: island SVG printed by fulgur 0.40.0 must show what
# was measured. It needs the fulgur CLI and poppler's pdftoppm; with
# SURIMONO_REQUIRE_FULGUR=1 it fails instead of skipping when they're missing.
if [ -f crates/gfx/tests/fulgur_print.rs ]; then
  step "gfx: fulgur print test"
  if ! command -v fulgur >/dev/null; then
    # fulgur-cli needs rustc 1.89 or later; the workspace's pinned toolchain is
    # only for our crates, so this one install uses stable.
    rustup toolchain list | grep -q '^stable' || rustup toolchain install stable --profile minimal
    RUSTUP_TOOLCHAIN=stable cargo install fulgur-cli --version =0.40.0 --locked
  fi
  if ! command -v pdftoppm >/dev/null && [ -n "${GITHUB_ACTIONS:-}" ]; then
    sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq poppler-utils
  fi
  SURIMONO_REQUIRE_FULGUR=1 cargo test -p surimono-gfx --test fulgur_print --locked; ran=$((ran + 1))
fi
# On a release branch (release/v*, the release PR): the release dry-run.
# Packages every crate and the gem; never publishes or tags.
case "${GITHUB_HEAD_REF:-${GITHUB_REF_NAME:-}}" in
  release/v*)
    step "release dry-run"; scripts/ci/release-dry-run.sh; ran=$((ran + 1)) ;;
esac
echo "nightly extra checks run: $ran"
