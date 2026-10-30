#!/usr/bin/env bash
# Runs the examples as CI does. From the repository root:
#
#   examples/run.sh            # Rust and Ruby examples, and the cross-binding check
#   examples/run.sh --no-ruby  # the Rust example only
#
# Needs examples/rust-islands in the workspace members, the test fonts in
# fixtures/fonts, and for Ruby the extension compiled
# (cd bindings/ruby && bundle exec rake compile:dev).
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD"

run_ruby=1
for a in "$@"; do
  case "$a" in
    --no-ruby) run_ruby=0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

echo "==> example: rust-islands"
cargo test --locked -p surimono-example-rust-islands

if [ "$run_ruby" = 1 ]; then
  echo "==> example: rails-snippet"
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/rails-snippet/render_report_test.rb")

  echo "==> example: the same report through Rust and Ruby gives the same bytes"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  cargo run --locked -q -p surimono-example-rust-islands -- \
    "$root/examples/shared/report.html" "$tmp/rust.html"
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/rails-snippet/render_report.rb" \
    --input "$root/examples/shared/report.html") > "$tmp/ruby.html"
  cmp "$tmp/rust.html" "$tmp/ruby.html"
fi

echo "==> examples ok"
