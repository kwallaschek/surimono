#!/usr/bin/env bash
# AddressSanitizer run of the test suite on nightly Rust (full.yml, nightly only).
set -euo pipefail
cd "$(dirname "$0")/../.."
rustup toolchain install nightly --profile minimal -c rust-src
RUSTFLAGS="-Zsanitizer=address" RUSTDOCFLAGS="-Zsanitizer=address" ASAN_OPTIONS="detect_leaks=1" \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu --workspace --exclude surimono-ruby --lib --tests
