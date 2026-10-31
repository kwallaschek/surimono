#!/usr/bin/env bash
# Runs exactly what .github/workflows/ci.yml runs on Linux: fmt, the offline
# fixture verifiers, clippy (warnings are errors), tests, the Ruby gem's tests and
# cargo-deny.
# Usage: scripts/ci.sh [--no-deny] [--no-ruby]
#   --no-ruby skips the Ruby extension (needs Ruby headers and libclang) and the gem tests.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/claude/target}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
. scripts/ci/lib.sh

run_deny=1; run_ruby=1
for a in "$@"; do
  case "$a" in
    --no-deny) run_deny=0 ;;
    --no-ruby) run_ruby=0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

# Locally every worktree shares one target dir, where another worktree's newer
# build of a workspace crate can look fresh here; rebuild ours from our sources.
# On GitHub each job has its own target dir.
if [ -z "${GITHUB_ACTIONS:-}" ] && [ -x scripts/fleet/touch-workspace.sh ]; then
  scripts/fleet/touch-workspace.sh
fi

# In CI (job.sh sets CI_JOB_OUT), the hostile-script suite writes its results
# there, so they land in ci-results for COMPAT.md (fixtures/tools/compat.py).
if [ -n "${CI_JOB_OUT:-}" ]; then
  mkdir -p "$CI_JOB_OUT"
  export SURIMONO_HOSTILE_RESULTS="${SURIMONO_HOSTILE_RESULTS:-$CI_JOB_OUT/hostile.json}"
fi

step "fmt";    cargo fmt --all -- --check
step "ci script tests"; python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'
scripts/ci/fixtures.sh
if [ "$run_ruby" = 1 ]; then
  ensure_libclang
  step "clippy (workspace incl. Ruby ext)"; cargo clippy --workspace --all-targets --locked -- -D warnings
  step "test (workspace)";                  cargo test --workspace --locked --no-fail-fast
  step "ruby gem (compile, test)"
  ruby_path
  (cd bindings/ruby && bundle install --quiet && bundle exec rake compile:dev test)
else
  step "clippy (default members)"; cargo clippy --all-targets --locked -- -D warnings
  step "test (default members)";   cargo test --locked --no-fail-fast
fi
if [ "$run_deny" = 1 ]; then
  if command -v cargo-deny >/dev/null; then
    step "cargo-deny"; cargo deny --locked check
  else
    echo "cargo-deny not installed: cargo install cargo-deny --locked" >&2; exit 1
  fi
fi
step "ok"
