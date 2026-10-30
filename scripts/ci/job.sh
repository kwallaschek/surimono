#!/usr/bin/env bash
# Runs one CI job command, teeing its output to ci-out/<name>/log.txt so
# publish-results.sh can push it to the ci-results branch.
# Usage: scripts/ci/job.sh <name> <command> [args...]
set -uo pipefail
name="$1"; shift
out="${CI_OUT:-ci-out}/${name}"
mkdir -p "$out"
export CI_JOB_OUT="$out"   # jobs may write extra results here (tables, images)
start=$(date +%s)
"$@" 2>&1 | tee "$out/log.txt"
code=${PIPESTATUS[0]}
end=$(date +%s)
printf '{"name":"%s","exit_code":%d,"seconds":%d}\n' "$name" "$code" "$((end - start))" > "$out/exit.json"
exit "$code"
