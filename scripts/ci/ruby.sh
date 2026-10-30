#!/usr/bin/env bash
# Builds the gem's native extension from source and runs the Ruby tests (full.yml, per Ruby version).
set -euo pipefail
cd "$(dirname "$0")/../../bindings/ruby"
bundle exec rake compile test
