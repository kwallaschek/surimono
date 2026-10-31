#!/usr/bin/env bash
# Full test run on macOS and aarch64 Linux (full.yml), including the Ruby extension.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_toolchain
ensure_libclang
rustc -Vv
# --no-fail-fast: every crate reports, not only those before the first failure.
cargo test --workspace --locked --no-fail-fast
