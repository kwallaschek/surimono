#!/usr/bin/env bash
# scripts/ci.sh for a worktree under the fleet's build lock and env. Agents use this,
# not ci.sh directly: two ci.sh runs at once race on the shared target dir, and a test
# timing out on a loaded machine isn't a result. cargo-deny isn't installed here, so it
# is skipped; GitHub CI runs it. Usage: scripts/fleet/wci.sh [--no-ruby]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/claude/target}"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export PATH="$here/strip-bin:$PATH"
mkdir -p "$CARGO_TARGET_DIR"
exec 9>"$CARGO_TARGET_DIR/.fleet-build-lock"
flock 9
"$here/../ci.sh" --no-deny "$@"
