#!/usr/bin/env bash
# fixtures/tools/page-check is a workspace of its own, with its own Cargo.lock, so the
# root workspace's fmt, clippy and tests don't reach it. Called from scripts/ci.sh;
# shares its target dir and environment.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
manifest=fixtures/tools/page-check/Cargo.toml
step "page-check: fmt";    cargo fmt --manifest-path "$manifest" --all -- --check
step "page-check: clippy"; cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
step "page-check: test";   cargo test --manifest-path "$manifest" --locked --no-fail-fast
