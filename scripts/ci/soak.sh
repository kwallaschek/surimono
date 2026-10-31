#!/usr/bin/env bash
# The soak test: 10,000 renders in one process, flat resident memory after
# warm-up. Soak tests are #[ignore]d tests whose name contains "soak"; they run
# in release, here and before every release. The Ruby extension has none.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_toolchain
cargo test --release --workspace --exclude surimono-ruby --locked -- --ignored soak --nocapture
