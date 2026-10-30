#!/usr/bin/env bash
# The soak test: 10,000 renders in one process, flat resident memory after warm-up.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo test --release --workspace --locked -- --ignored soak --nocapture
