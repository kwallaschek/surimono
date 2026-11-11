#!/usr/bin/env bash
# Marks this orchestrator session as active, so a scheduled session doesn't start a
# second fleet. Writes heartbeat.json on the fleet-heartbeat branch.
# Usage: scripts/fleet/heartbeat.sh [note]     |     scripts/fleet/heartbeat.sh --age
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ "${1:-}" = "--age" ]; then
  git fetch --quiet origin fleet-heartbeat 2>/dev/null || { echo "none"; exit 0; }
  git show origin/fleet-heartbeat:heartbeat.json | python3 -c "
import sys,json,datetime
x=json.load(sys.stdin)
t=datetime.datetime.fromisoformat(x['time'].replace('Z','+00:00'))
age=(datetime.datetime.now(datetime.timezone.utc)-t).total_seconds()/60
print(f\"{age:.0f} minutes ago by {x.get('session','?')}: {x.get('note','')}\")"
  exit 0
fi
note="${1:-working}"
# Refuses (exit 3) when another live session holds the fleet; see owner-check.sh.
scripts/fleet/owner-check.sh
# Agents may send heartbeats at the same moment: hold a lock, and use a branch and
# worktree of our own that are removed on any exit.
exec 9>"${TMPDIR:-/tmp}/surimono-heartbeat.lock"
flock -w 120 9
work="$(mktemp -d)"
br="hb-tmp-$$"
cleanup() { git worktree remove --force "$work" >/dev/null 2>&1 || true; git branch -D "$br" >/dev/null 2>&1 || true; git worktree prune >/dev/null 2>&1 || true; }
trap cleanup EXIT
git worktree prune
git worktree add --quiet --detach "$work" HEAD
(
  cd "$work"
  git checkout --quiet --orphan "$br"
  git rm -rfq --ignore-unmatch . && find . -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
  printf '{"time":"%s","session":"%s","note":"%s"}\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${CLAUDE_CODE_REMOTE_SESSION_ID:-unknown}" "$note" > heartbeat.json
  git add heartbeat.json
  git -c user.name="surimono fleet" -c user.email="noreply@anthropic.com" commit --quiet -m "heartbeat: $note"
  git push --quiet --force origin HEAD:refs/heads/fleet-heartbeat
)
echo "heartbeat sent"
