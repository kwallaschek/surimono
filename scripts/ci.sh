#!/usr/bin/env bash
# Runs exactly what .github/workflows/ci.yml runs on Linux.
# Usage: scripts/ci.sh [--no-deny] [--no-ruby]
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/claude/target}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

run_deny=1; run_ruby=1
for a in "$@"; do
  case "$a" in
    --no-deny) run_deny=0 ;;
    --no-ruby) run_ruby=0 ;;
  esac
done

step() { printf '\n==> %s\n' "$*"; }

step "fmt";    cargo fmt --all -- --check
if [ "$run_ruby" = 1 ]; then
  step "clippy (workspace incl. Ruby ext)"; cargo clippy --workspace --all-targets --locked -- -D warnings
  step "test (workspace)";                  cargo test --workspace --locked
else
  step "clippy (default members)"; cargo clippy --all-targets --locked -- -D warnings
  step "test (default members)";   cargo test --locked
fi
if [ "$run_deny" = 1 ]; then
  if command -v cargo-deny >/dev/null; then
    step "cargo-deny"; cargo deny --locked check
  else
    echo "cargo-deny not installed: cargo install cargo-deny --locked" >&2; exit 1
  fi
fi
step "ok"
