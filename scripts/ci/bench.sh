#!/usr/bin/env bash
# The Grover benchmark (bench.yml). The harness lives in bench/ (owned by the
# conformance researcher); this only calls it.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
# bench/rust is its own workspace (toolchain 1.97.0, for fulgur's krilla). Build
# it under the main target dir, which the workflow's rust-cache saves (nested
# target dirs are kept), so a cold release build doesn't eat the job's timeout.
export BENCH_RUST_TARGET_DIR="${BENCH_RUST_TARGET_DIR:-${CARGO_TARGET_DIR:-$REPO_ROOT/target}/bench-rust}"
if [ ! -x bench/run.sh ]; then echo "bench/run.sh not there yet; nothing to run"; exit 0; fi
bench/run.sh "${CI_JOB_OUT:-ci-out/bench}"
