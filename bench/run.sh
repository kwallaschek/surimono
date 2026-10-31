#!/usr/bin/env bash
# The Grover benchmark: time and peak memory, Grover (Chrome) against surimono +
# fulgur, on bench/report/report.html.
#
#   bench/run.sh <outdir> [bench.py options, e.g. --runs 5 --systems grover,fulgur-prerendered]
#
# Writes <outdir>/summary.md (the table) and <outdir>/results.json (every run).
# Installs what it needs first: the pinned npm packages (fixtures/tools for the
# chart libraries and Playwright's Chromium, bench/grover for Puppeteer) and the
# pinned gems (bench/Gemfile.lock: grover, fulgur) into bench/.cache.
#
# On a CI runner ($CI set) it also installs Playwright's Chromium (the build the
# Chrome references use) and its chrome-headless-shell, poppler-utils for page
# counts, bench/rust's toolchain, and builds the surimono gem from bindings/ruby
# (release) when it has a Surimono::Renderer. PDFs and pages go to a work
# directory outside <outdir> in CI, so only the summary and the JSON are
# published.
#
# CI time: bench.yml gives the job 60 minutes. bench.py stops starting new work
# BENCH_TIME_BUDGET_MIN (default 50 in CI) after this script started, and
# rewrites results.json and summary.md after every phase, so whatever was
# measured is published even when a cold bench/rust build eats the time. The
# build itself gets at most BENCH_RUST_BUILD_TIMEOUT seconds (default 1800) and
# never the last 15 minutes. It builds into $CARGO_TARGET_DIR/bench-rust, inside
# the target dir Swatinem/rust-cache saves.
set -euo pipefail
export BENCH_START_EPOCH="${BENCH_START_EPOCH:-$(date +%s)}"
cd "$(dirname "$0")/.."
root="$(pwd)"
out="${1:?usage: bench/run.sh <outdir> [bench.py options]}"; shift
mkdir -p "$out"
out="$(cd "$out" && pwd)"

step() { printf '\n==> %s\n' "$*"; }

step "fixture tools (chart libraries, Playwright)"
if [ -n "${CI:-}" ] || [ ! -d fixtures/tools/node_modules ]; then
  (cd fixtures/tools && npm ci --no-audit --no-fund)
fi
if [ -n "${CI:-}" ]; then
  # Chromium and its chrome-headless-shell (the grover-shell row), same build.
  (cd fixtures/tools && npx --no-install playwright install --with-deps chromium)
  if ! command -v pdfinfo >/dev/null; then sudo apt-get update -qq && sudo apt-get install -y -qq poppler-utils; fi
  export BENCH_TIME_BUDGET_MIN="${BENCH_TIME_BUDGET_MIN:-50}"
  # bench/rust builds inside the target dir rust-cache saves (bench.yml sets
  # CARGO_TARGET_DIR=target, relative to the repo root).
  ci_target="${CARGO_TARGET_DIR:-target}"
  case "$ci_target" in /*) ;; *) ci_target="$root/$ci_target" ;; esac
  export BENCH_RUST_TARGET_DIR="${BENCH_RUST_TARGET_DIR:-$ci_target/bench-rust}"
  step "bench/rust toolchain (bench/rust/rust-toolchain.toml)"
  (cd bench/rust && rustup toolchain install)
fi

step "reference report is what its generator writes"
node bench/report/gen-report.mjs --check

step "puppeteer for Grover (no browser download: Grover uses Playwright's Chromium)"
if [ -n "${CI:-}" ] || [ ! -d bench/grover/node_modules ]; then
  (cd bench/grover && PUPPETEER_SKIP_DOWNLOAD=1 npm ci --no-audit --no-fund)
fi

step "gems (grover, fulgur)"
export BUNDLE_GEMFILE="$root/bench/Gemfile"
export BUNDLE_PATH="${BUNDLE_PATH:-$root/bench/.cache/bundle}"
bundle install --quiet

if [ -n "${CI:-}" ] && [ -z "${SURIMONO_RUBY_LIB:-}" ] && grep -q "class Renderer" bindings/ruby/lib/surimono.rb 2>/dev/null; then
  step "surimono gem (release build from bindings/ruby)"
  ( unset BUNDLE_GEMFILE BUNDLE_PATH
    cd bindings/ruby && bundle install --quiet && bundle exec rake compile ) \
    || echo "surimono gem did not build; the surimono-ruby row will say so"
fi

work_args=()
if [ -n "${CI:-}" ]; then work_args=(--work "${RUNNER_TEMP:-/tmp}/surimono-bench-work"); fi

step "benchmark"
python3 bench/tools/bench.py --out "$out" "${work_args[@]}" "$@"
