#!/usr/bin/env bash
# Opens a pull request into main. Usage: scripts/fleet/open-pr.sh <branch> <title> <body-markdown-file>
set -euo pipefail
here="$(dirname "$0")"
branch="$1"; title="$2"; bodyfile="$3"
python3 - "$branch" "$title" "$bodyfile" > /tmp/pr-$$.json <<'PY'
import json, sys
branch, title, bodyfile = sys.argv[1:4]
import os
sid = os.environ.get("CLAUDE_CODE_REMOTE_SESSION_ID", "").replace("cse_", "session_", 1)
body = open(bodyfile).read().rstrip() + "\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n"
if sid:
    body += f"\nhttps://claude.ai/code/{sid}\n"
print(json.dumps({"title": title, "head": branch, "base": "main", "body": body}))
PY
"$here/gh-api.sh" POST /pulls /tmp/pr-$$.json | python3 -c "import sys,json; x=json.load(sys.stdin); print(x.get('html_url') or x)"
rm -f /tmp/pr-$$.json
