#!/usr/bin/env bash
# AddressSanitizer run of the test suite on nightly Rust (full.yml, linux-extra).
# Flags as in the Unstable Book (compiler-flags/sanitizer, checked 2026-10-31):
# -Zsanitizer=address for the code and an instrumented std via -Zbuild-std, with an
# explicit --target so build scripts and proc macros are built without ASan.
# The Ruby extension is excluded: it links libruby, which isn't built with ASan.
# Set ASAN_TOOLCHAIN to pin a dated nightly if a new one breaks the build.
set -euo pipefail
cd "$(dirname "$0")/../.."
toolchain="${ASAN_TOOLCHAIN:-nightly}"
target="$(uname -m)-unknown-linux-gnu"
rustup toolchain install "$toolchain" --profile minimal -c rust-src
rustc "+$toolchain" -Vv
export RUSTFLAGS="-Zsanitizer=address" RUSTDOCFLAGS="-Zsanitizer=address"
export ASAN_OPTIONS="detect_leaks=1"
cargo "+$toolchain" test -Zbuild-std --target "$target" --locked \
  --workspace --exclude surimono-ruby --lib --tests
