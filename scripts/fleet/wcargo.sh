#!/usr/bin/env bash
# Runs cargo for this worktree with the fleet's env, holding a lock across the touch
# and the whole cargo command, so another worktree can't slip its artifacts in between
# (touch-workspace.sh alone lost that race during concurrent builds on 4 Nov).
# Waiting here is normal. Usage: scripts/fleet/wcargo.sh test -p surimono-dom
set -euo pipefail
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/claude/target}"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export PATH="$(cd "$(dirname "$0")" && pwd)/strip-bin:$PATH"   # disk: see strip-bin/cc
mkdir -p "$CARGO_TARGET_DIR"
exec 9>"$CARGO_TARGET_DIR/.fleet-build-lock"
flock 9
"$(dirname "$0")/touch-workspace.sh"
cargo "$@"
