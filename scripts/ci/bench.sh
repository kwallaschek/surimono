#!/usr/bin/env bash
# The Grover benchmark (bench.yml). The harness lives in bench/ (owned by the
# conformance researcher); this only calls it.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ ! -x bench/run.sh ]; then echo "bench/run.sh not there yet; nothing to run"; exit 0; fi
bench/run.sh "${CI_JOB_OUT:-ci-out/bench}"
