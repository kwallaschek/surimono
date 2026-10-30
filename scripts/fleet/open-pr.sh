#!/usr/bin/env bash
# Opens a pull request into main. Usage: scripts/fleet/open-pr.sh <branch> <title> <body-markdown-file>
set -euo pipefail
here="$(dirname "$0")"
branch="$1"; title="$2"; bodyfile="$3"
python3 - "$branch" "$title" "$bodyfile" > /tmp/pr-$$.json <<'PY'
import json, sys
branch, title, bodyfile = sys.argv[1:4]
body = open(bodyfile).read().rstrip() + "\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n\nhttps://claude.ai/code/session_011c8hfp9aeYhQp5RpWyywaN\n"
print(json.dumps({"title": title, "head": branch, "base": "main", "body": body}))
PY
"$here/gh-api.sh" POST /pulls /tmp/pr-$$.json | python3 -c "import sys,json; x=json.load(sys.stdin); print(x.get('html_url') or x)"
rm -f /tmp/pr-$$.json
