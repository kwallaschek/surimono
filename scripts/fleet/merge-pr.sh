#!/usr/bin/env bash
# Merges a PR with a merge commit (so stacked branches stay valid). MERGE_METHOD=squash
# squashes instead, with the PR title and body as the message: only for PRs whose
# commits mention unreported engine bugs (DECISIONS 2026-10-31).
# Only after CI is green on the head and a fresh reviewer posted REVIEW: APPROVE
# for that head. Kay authorised fleet merges on 2026-10-31.
# Usage: scripts/fleet/merge-pr.sh <number> [expected-head-sha-prefix]
set -euo pipefail
here="$(dirname "$0")"
n="$1"; expect="${2:-}"; method="${MERGE_METHOD:-merge}"
"$here/owner-check.sh"   # exit 3 when another orchestrator session holds the fleet
"$here/gh-api.sh" GET "/pulls/$n" > /tmp/pr-$$.json
head=$(python3 -c "import sys,json; print(json.load(open(sys.argv[1]))['head']['sha'])" /tmp/pr-$$.json)
if [ -n "$expect" ] && [ "${head#"$expect"}" = "$head" ]; then
  echo "PR #$n head is ${head:0:9}, not the reviewed $expect; not merging" >&2; exit 1
fi
python3 - "$n" "$head" "$method" /tmp/pr-$$.json > /tmp/merge-$$.json <<'PY'
import json, sys
n, head, method, pr = sys.argv[1], sys.argv[2], sys.argv[3], json.load(open(sys.argv[4]))
if method == "squash":
    body = {"merge_method": "squash", "sha": head, "commit_title": f"{pr['title']} (#{n})",
            "commit_message": pr.get("body") or ""}
else:
    body = {"merge_method": "merge", "sha": head, "commit_title": f"Merge pull request #{n}"}
print(json.dumps(body))
PY
"$here/gh-api.sh" PUT "/pulls/$n/merge" /tmp/merge-$$.json | python3 -c "import sys,json; x=json.load(sys.stdin); print('merged' if x.get('merged') else 'NOT merged', x.get('sha','')[:9], x.get('message'))"
rm -f /tmp/merge-$$.json /tmp/pr-$$.json
