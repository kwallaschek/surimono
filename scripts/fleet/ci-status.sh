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
    print(f\"{r['name']:40} {r['status']:12} {r.get('conclusion') or '-'}  id={r['id']}\")
print('total', x.get('total_count'))"
echo "== annotations of failed runs (useful when a job failed before its script ran)"
"$here/gh-api.sh" GET "/commits/$sha/check-runs?per_page=100&status=completed" | python3 -c "
import sys,json
x=json.load(sys.stdin)
print(' '.join(str(r['id']) for r in x.get('check_runs',[]) if r.get('conclusion')=='failure'))" | tr ' ' '\n' | while read -r id; do
  [ -n "$id" ] || continue
  "$here/gh-api.sh" GET "/check-runs/$id/annotations" | python3 -c "
import sys,json
for a in json.load(sys.stdin):
    if a.get('annotation_level')=='failure': print('  ', '$id', a.get('message','')[:300])"
done
echo "== commit statuses"
"$here/gh-api.sh" GET "/commits/$sha/status" | python3 -c "
import sys,json
x=json.load(sys.stdin)
for s in x.get('statuses',[]): print(f\"{s['context']:40} {s['state']:10} {s.get('description') or ''}\")
print('combined', x.get('state'))"
echo "== ci-results"
git fetch --quiet origin ci-results 2>/dev/null && git ls-tree -r --name-only origin/ci-results "runs/$sha" 2>/dev/null | grep status.json | while read -r f; do git show "origin/ci-results:$f"; done
