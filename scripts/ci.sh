#!/usr/bin/env bash
# Runs exactly what .github/workflows/ci.yml runs on Linux: fmt, the offline
# fixture verifiers, clippy (warnings are errors), tests, clippy and the tests of
# surimono-dom again with its `raikiri` feature, the WPT runner's own fmt,
# clippy and tests, page-check's, the Ruby gem's tests, the examples
# (examples/run.sh, skipped while it doesn't exist) and cargo-deny.
# Usage: scripts/ci.sh [--no-deny] [--no-ruby]
#   --no-ruby skips the Ruby extension (needs Ruby headers and libclang), the gem
#             tests and the Ruby examples.
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
  run_examples
else
  step "clippy (default members)"; cargo clippy --all-targets --locked -- -D warnings
  step "test (default members)";   cargo test --locked --no-fail-fast
  run_examples --no-ruby
fi
# surimono-dom's `raikiri` feature (the Raikiri layout engine, E3-R1) is off by
# default, so nothing above builds it: clippy never reads the code behind it and
# crates/dom/tests/raikiri_build.rs compiles to 0 tests. Lint and test that one
# crate again with the feature on (P2-PKG-09). Two more cargo runs for one
# package, not `--workspace --all-features`: that would rebuild every crate and
# turn on the other crates' test-only features (surimono-core's test-hooks,
# surimono-islands' test-estimate). Cargo resolves features per run, so the
# default-feature binaries above link nothing of Raikiri. No option skips this:
# it is the only check the feature gets.
step "clippy (surimono-dom, raikiri feature)"
cargo clippy -p surimono-dom --all-targets --features raikiri --locked -- -D warnings
step "test (surimono-dom, raikiri feature)"
cargo test -p surimono-dom --features raikiri --locked --no-fail-fast
scripts/ci/wpt-runner.sh
scripts/ci/page-check.sh
if [ "$run_deny" = 1 ]; then
  if command -v cargo-deny >/dev/null; then
    step "cargo-deny"; cargo deny --locked check
  else
    echo "cargo-deny not installed: cargo install cargo-deny --locked" >&2; exit 1
  fi
fi
step "ok"
