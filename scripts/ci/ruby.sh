#!/usr/bin/env bash
# Ruby gem on one Ruby version (full.yml matrix, 3.2 to 4.0):
#   1. compile the extension in release and run the tests from the repository;
#   2. build the source gem from the staged tree, install it into an empty
#      GEM_HOME (compiling it as `gem install` would) and run the tests against it.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_toolchain
ensure_libclang
ruby_path
ruby -v
cd bindings/ruby
bundle install --quiet
step "compile and test (repository)"
bundle exec rake compile test
step "source gem: build, install, test"
# Reuse the compiled dependencies from step 1 for the installed gem's build.
export RB_SYS_CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
bundle exec rake gem:verify_source
mkdir -p "${CI_JOB_OUT:-$REPO_ROOT/ci-out}"
ls -la pkg/ | tee "${CI_JOB_OUT:-$REPO_ROOT/ci-out}/gems.txt"
