#!/usr/bin/env bash
# Runs the examples and checks them. From the repository root:
#
#   examples/run.sh            # Rust and Ruby examples, and the cross-binding check
#   examples/run.sh --no-ruby  # the Rust example only
#
# CI runs it in full from scripts/ci.sh (every pull request) and from
# scripts/ci/ruby.sh (Ruby 3.2 to 4.0 on main, nightly and release branches).
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
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/rails-snippet/render_page_test.rb")

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  echo "==> example: the checkpoint-3 script on a page of ours"
  cp "$root/examples/shared/page.html" "$tmp/try.html"
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/checkpoint-3/try_template.rb" \
    "$tmp/try.html" --assets "$root/examples/shared") | tee "$tmp/try.txt"
  grep -qx 'settled: idle' "$tmp/try.txt"
  grep -qx 'not loaded: 0' "$tmp/try.txt"
  grep -qx 'layout queries: none' "$tmp/try.txt"
  test -s "$tmp/try.out.html"

  echo "==> example: the same report through Rust and Ruby gives the same bytes"
  cargo run --locked -q -p surimono-example-rust-islands -- \
    "$root/examples/shared/report.html" "$tmp/rust.html"
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/rails-snippet/render_report.rb" \
    --input "$root/examples/shared/report.html") > "$tmp/ruby.html"
  cmp "$tmp/rust.html" "$tmp/ruby.html"

  echo "==> example: the same page through Rust and Ruby gives the same bytes"
  cargo run --locked -q -p surimono-example-rust-islands --bin page -- \
    "$root/examples/shared/page.html" "$tmp/rust-page.html"
  (cd bindings/ruby && bundle exec ruby -Ilib "$root/examples/rails-snippet/render_page.rb" \
    --input "$root/examples/shared/page.html") > "$tmp/ruby-page.html"
  cmp "$tmp/rust-page.html" "$tmp/ruby-page.html"
fi

echo "==> examples ok"
