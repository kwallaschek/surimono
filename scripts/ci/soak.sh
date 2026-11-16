#!/usr/bin/env bash
# The soak tests: 10,000 renders in one process, flat resident memory after
# warm-up. Soak tests are #[ignore]d tests whose name contains "soak"; they run
# in release, here and before every release. The Ruby extension has none.
# Each run must pass at least the soaks it has today (counted_cargo_test): a
# filter that matches nothing passes in cargo but fails here. Building something
# new here while Cargo.lock and the manifests stay as they are: raise
# `ci-cache-epoch` in crates/cli/Cargo.toml in the same pull request.
# scripts/ci/test_ci_feature_steps.py runs this script against a stub cargo.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_toolchain
# Default features: the DOM (tests/soak.rs, 1), islands (tests/soak.rs, 4) and
# pages (tests/page_soak.rs, 1). Raise --min-passed when a soak is added.
step "soak (workspace, default features)"
counted_cargo_test --target soak --target page_soak --min-passed 6 -- \
  --release --workspace --exclude surimono-ruby --locked -- --ignored soak --nocapture
# surimono-dom's off-by-default `raikiri` feature: 10,000 renders with two
# Raikiri layouts each (crates/dom/tests/layout_soak.rs, P2-PKG-11; 163 to 171 s
# on the fleet's 2-core machine). The run above builds surimono-dom without it.
step "soak (surimono-dom, raikiri feature: layout_soak)"
counted_cargo_test --target layout_soak --min-passed 1 -- \
  --release -p surimono-dom --features raikiri --locked --test layout_soak \
  -- --ignored soak --nocapture
