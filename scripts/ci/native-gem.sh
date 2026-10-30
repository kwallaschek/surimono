#!/usr/bin/env bash
# Cross-compiles a precompiled native gem with rb-sys-dock (full.yml, per platform).
# Usage: scripts/ci/native-gem.sh <platform>   e.g. x86_64-linux, arm64-darwin
set -euo pipefail
platform="$1"
rubies="${RUBY_VERSIONS:-3.2,3.3,3.4,4.0}"
cd "$(dirname "$0")/../../bindings/ruby"
bundle install --quiet
bundle exec rb-sys-dock --platform "$platform" --ruby-versions "$rubies" --build
mkdir -p "${CI_JOB_OUT:-../../ci-out}"
ls -la pkg/ | tee "${CI_JOB_OUT:-../../ci-out}/gems.txt"
