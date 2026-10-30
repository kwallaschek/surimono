#!/usr/bin/env bash
# Full test run on macOS and aarch64 Linux (full.yml).
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo test --workspace --locked
