#!/usr/bin/env bash
# Shows CI state for a commit: GitHub check runs plus the logs published to ci-results.
# Usage: scripts/fleet/ci-status.sh <sha>
set -uo pipefail
here="$(dirname "$0")"
sha="$1"
echo "== check runs"
"$here/gh-api.sh" GET "/commits/$sha/check-runs?per_page=100" | python3 -c "
import sys,json
x=json.load(sys.stdin)
for r in x.get('check_runs',[]):
    print(f\"{r['name']:40} {r['status']:12} {r.get('conclusion') or '-'}\")
print('total', x.get('total_count'))"
echo "== commit statuses"
"$here/gh-api.sh" GET "/commits/$sha/status" | python3 -c "
import sys,json
x=json.load(sys.stdin)
for s in x.get('statuses',[]): print(f\"{s['context']:40} {s['state']:10} {s.get('description') or ''}\")
print('combined', x.get('state'))"
echo "== ci-results"
git fetch --quiet origin ci-results 2>/dev/null && git ls-tree -r --name-only origin/ci-results "runs/$sha" 2>/dev/null | grep status.json | while read -r f; do git show "origin/ci-results:$f"; done
