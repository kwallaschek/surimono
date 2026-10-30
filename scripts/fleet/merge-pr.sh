#!/usr/bin/env bash
# Merges a PR with a merge commit (never squash, so stacked branches stay valid).
# Only after CI is green on the head and a fresh reviewer posted REVIEW: APPROVE
# for that head. Kay authorised fleet merges on 2026-10-31.
# Usage: scripts/fleet/merge-pr.sh <number> [expected-head-sha-prefix]
set -euo pipefail
here="$(dirname "$0")"
n="$1"; expect="${2:-}"
head=$("$here/gh-api.sh" GET "/pulls/$n" | python3 -c "import sys,json; print(json.load(sys.stdin)['head']['sha'])")
if [ -n "$expect" ] && [ "${head#"$expect"}" = "$head" ]; then
  echo "PR #$n head is ${head:0:9}, not the reviewed $expect; not merging" >&2; exit 1
fi
python3 - "$n" "$head" > /tmp/merge-$$.json <<'PY'
import json, sys
print(json.dumps({"merge_method": "merge", "sha": sys.argv[2], "commit_title": f"Merge pull request #{sys.argv[1]}"}))
PY
"$here/gh-api.sh" PUT "/pulls/$n/merge" /tmp/merge-$$.json | python3 -c "import sys,json; x=json.load(sys.stdin); print('merged' if x.get('merged') else 'NOT merged', x.get('sha','')[:9], x.get('message'))"
rm -f /tmp/merge-$$.json
