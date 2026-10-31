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
# The real-world suite checks its pinned packages against registry.npmjs.org
# (the PR gate checks only the embedded files, offline).
if [ -f fixtures/real-world/verify.py ]; then
  step "fixtures: real-world suite (needs registry.npmjs.org)"
  python3 -I fixtures/real-world/verify.py; ran=$((ran + 1))
fi
# The fulgur print test: island SVG printed by fulgur 0.40.0 must show what
# was measured. It needs the fulgur CLI and poppler's pdffonts and pdftotext
# (poppler-utils); with SURIMONO_REQUIRE_FULGUR=1 it fails instead of skipping
# when they're missing.
if [ -f crates/gfx/tests/fulgur_print.rs ]; then
  step "gfx: fulgur print test"
  if ! command -v fulgur >/dev/null; then
    # fulgur-cli needs rustc 1.89 or later; the workspace's pinned toolchain is
    # only for our crates, so this one install uses stable.
    rustup toolchain list | grep -q '^stable' || rustup toolchain install stable --profile minimal
    # Its own target dir: stable-toolchain artifacts stay out of the
    # workspace's cached one.
    RUSTUP_TOOLCHAIN=stable cargo install fulgur-cli --version =0.40.0 --locked \
      --target-dir "$(mktemp -d)"
  fi
  if { ! command -v pdffonts >/dev/null || ! command -v pdftotext >/dev/null; } && [ -n "${GITHUB_ACTIONS:-}" ]; then
    sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq poppler-utils
  fi
  SURIMONO_REQUIRE_FULGUR=1 cargo test -p surimono-gfx --test fulgur_print --locked; ran=$((ran + 1))
fi
# The full out-of-memory sweep (P1-RT-15, #45): every allocation of a script
# fails once in turn. Slow, so #[ignore]d; the ASan job runs it too, with
# quickjs.c's arena off. A no-op until the test exists.
if grep -rq 'fn out_of_memory_full_sweep' crates/core/src 2>/dev/null; then
  step "core: out-of-memory full sweep (ignored test)"
  cargo test -p surimono-core --lib --locked -- --ignored out_of_memory_full_sweep; ran=$((ran + 1))
fi
echo "nightly extra checks run: $ran"
