#!/usr/bin/env bash
# Marks this worktree's workspace crates as changed so cargo rebuilds them from
# THIS worktree's sources. Needed because all worktrees share one CARGO_TARGET_DIR
# and cargo names workspace-crate artifacts by their path relative to the
# workspace root, so another worktree's newer build can look fresh here.
# Dependencies from crates.io are unaffected. Run before any local build or test.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
find crates bindings examples vendor -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name '*.js' -o -name '*.c' -o -name '*.h' \) \
  -not -path '*/node_modules/*' -not -path '*/target/*' -print0 2>/dev/null | xargs -0 -r touch
