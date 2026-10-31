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
status=0
# --no-fail-fast: every crate reports, not only those before the first failure.
cargo "+$toolchain" test -Zbuild-std --target "$target" --locked --no-fail-fast \
  --workspace --exclude surimono-ruby --lib --tests || status=$?

# The full out-of-memory sweep (P1-RT-15): every allocation QuickJS makes
# during a script fails once in turn. Under ASan, surimono-core's build.rs
# compiles quickjs.c without its arena, so the sweep reaches every
# allocation. Slow, so #[ignore]d and run only here; a no-op until the test
# exists.
if grep -rq 'fn out_of_memory_full_sweep' crates/core/src 2>/dev/null; then
  printf '\n==> core: out-of-memory full sweep (ignored test)\n'
  cargo "+$toolchain" test -Zbuild-std --target "$target" --locked \
    -p surimono-core --lib out_of_memory_full_sweep -- --ignored || status=$?
else
  echo "out_of_memory_full_sweep isn't in crates/core yet; skipping"
fi
exit "$status"
