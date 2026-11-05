#!/usr/bin/env bash
# The WPT runner is a workspace of its own, with its own Cargo.lock (wpt/runner), so
# the root workspace's fmt, clippy and tests don't reach it. scripts/ci/wpt.sh only
# runs it. Called from scripts/ci.sh; shares its target dir and environment.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
manifest=wpt/runner/Cargo.toml
step "wpt runner: fmt";    cargo fmt --manifest-path "$manifest" --all -- --check
step "wpt runner: clippy"; cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
step "wpt runner: test";   cargo test --manifest-path "$manifest" --locked --no-fail-fast
