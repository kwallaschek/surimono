#!/usr/bin/env bash
# Records a build job's result next to its logs (ci-out/job.json), for the
# workflow's publish job (docs/ci/*.yml), which uploads nothing itself.
# Usage (an always() step): scripts/ci/job-status.sh "${{ job.status }}"
set -uo pipefail
out="${CI_OUT:-ci-out}"
mkdir -p "$out"
# Plain path-segment characters only; publish-results.sh reads them back with sed.
clean() { printf '%s' "$1" | tr -cd 'A-Za-z0-9._-'; }
printf '{"job":"%s","matrix":"%s","status":"%s"}\n' \
  "$(clean "${GITHUB_JOB:-}")" "$(clean "${MATRIX_KEY:-}")" "$(clean "${1:-unknown}")" > "$out/job.json"
echo "recorded $(cat "$out/job.json")"
