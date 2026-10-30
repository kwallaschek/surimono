#!/usr/bin/env bash
# Writes THIRD_PARTY_LICENSES.txt: the licences and notices of everything the
# Ruby gem's native library statically links (the Rust crates, found by
# cargo-about from Cargo.lock), then those of the JavaScript libraries built
# into it (crates/islands/bundles/*/LICENSE*, NOTICE*).
# Usage: scripts/ci/third-party-licenses.sh <out-file>
set -euo pipefail
out="$(realpath -m "$1")"
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_cargo_about

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
# Fails if any crate's licence isn't in scripts/ci/about.toml's accepted list.
cargo about generate --locked \
  --manifest-path bindings/ruby/ext/surimono/Cargo.toml \
  -c scripts/ci/about.toml scripts/ci/about.hbs -o "$tmp"

{
  cat "$tmp"
  for dir in crates/islands/bundles/*/; do
    [ -d "$dir" ] || continue
    name="$(basename "$dir")"
    for f in "$dir"LICENSE* "$dir"NOTICE* "$dir"COPYING*; do
      [ -f "$f" ] || continue
      printf '\n================================================================================\n'
      printf 'JavaScript library built in: %s (%s)\n\n' "$name" "$(basename "$f")"
      cat "$f"
    done
  done
} > "$out"

# MPL-2.0 (section 3.2) wants recipients told where the source is; the
# template says so whenever an MPL-2.0 crate is listed. Check it did.
if grep -q '(MPL-2.0)$' "$out" && ! grep -q 'Source Code Form is available' "$out"; then
  echo "MPL-2.0 crates are listed but the source-availability notice is missing" >&2
  exit 1
fi
echo "wrote $out ($(grep -c '' "$out") lines; $(grep -c '^JavaScript library built in' "$out") JS licence files)"
