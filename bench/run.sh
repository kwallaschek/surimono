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
# Chrome references use), poppler-utils for page counts, and builds the
# surimono gem from bindings/ruby (release) when it has a Surimono::Renderer.
# PDFs and pages go to a work directory outside <outdir> in CI, so only the
# summary and the JSON are published.
set -euo pipefail
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
  (cd fixtures/tools && npx --no-install playwright install --with-deps --no-shell chromium)
  if ! command -v pdfinfo >/dev/null; then sudo apt-get update -qq && sudo apt-get install -y -qq poppler-utils; fi
fi

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
