#!/usr/bin/env bash
# Stops a second orchestrator session from running the fleet at the same time.
# The session named in heartbeat.json (fleet-heartbeat branch) holds the fleet while
# its heartbeat is under 3 hours old. Any other session gets a STOP message and exit 3.
# A heartbeat that names no session ("unknown", from older scripts) holds nothing.
# FLEET_TAKEOVER=1 skips the check: only the session Kay is talking to uses it.
# Usage: scripts/fleet/owner-check.sh   (heartbeat.sh and merge-pr.sh call it)
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "${FLEET_TAKEOVER:-}" = 1 ] && exit 0
me="${CLAUDE_CODE_REMOTE_SESSION_ID:-unknown}"
if [ -n "${FLEET_HEARTBEAT_JSON:-}" ]; then
  hb="$FLEET_HEARTBEAT_JSON"   # for testing this script
else
  git fetch --quiet origin fleet-heartbeat 2>/dev/null || exit 0
  hb="$(git show origin/fleet-heartbeat:heartbeat.json 2>/dev/null)" || exit 0
fi
printf '%s' "$hb" | python3 -c '
import sys, json, datetime
me = sys.argv[1]
x = json.load(sys.stdin)
holder = x.get("session") or "unknown"
t = datetime.datetime.fromisoformat(x["time"].replace("Z", "+00:00"))
age = (datetime.datetime.now(datetime.timezone.utc) - t).total_seconds() / 60
note = x.get("note", "")
if holder != "unknown" and holder != me and age < 180:
    print(f"STOP: another orchestrator session ({holder}) is running the fleet; "
          f"its heartbeat is {age:.0f} min old: {note}.", file=sys.stderr)
    print("Change nothing more: no merges, pushes or messages. Stop your subagents and end "
          "this session with one line saying the fleet is already running.", file=sys.stderr)
    sys.exit(3)
' "$me"
