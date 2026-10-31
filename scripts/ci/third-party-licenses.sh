#!/usr/bin/env bash
# Writes THIRD_PARTY_LICENSES.txt: the licences and notices of everything the
# Ruby gem's native library statically links (the Rust crates, found by
# cargo-about from Cargo.lock), then those of the JavaScript libraries built
# into it (every licence and notice file in crates/islands/bundles/*/).
# Usage: scripts/ci/third-party-licenses.sh <out-file>
#
# It fails, rather than write an incomplete or wrong file, when:
# - a crate's licence isn't accepted in scripts/ci/about.toml, or can't be
#   read (cargo-about --fail); MPL-2.0 is accepted only for the crates named
#   there, exactly the shipped MPL-2.0 crates and all of them among
#   deny.toml's exceptions (check-mpl.py);
# - an MPL-2.0 crate doesn't come unmodified from crates.io (a [patch], path
#   or git source), since the notice promises its source is published there;
# - a bundle directory holds a file that is neither JavaScript nor a licence
#   or notice this script copies, so a new bundle can't ship without its
#   licences.
# It needs python3 3.11 or later (check-mpl.py reads TOML with tomllib).
# cargo-about runs with --frozen: no network, so licence texts never come
# from a crate's repository at generation time. Crate sources are fetched
# first, from the checksums in Cargo.lock.
set -euo pipefail
out="$(realpath -m "$1")"
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_cargo_about

manifest=bindings/ruby/ext/surimono/Cargo.toml
cargo fetch --locked --manifest-path "$manifest" >&2

tmp="$(mktemp)"
shipped="$(mktemp)"
# Written next to the output, then moved into place, so a failure never leaves
# a partial file where gem staging could pick it up.
partial="$(mktemp "$(dirname "$out")/.THIRD_PARTY_LICENSES.XXXXXX")"
trap 'rm -f "$tmp" "$shipped" "$partial"' EXIT

# The crates the shipped library links: normal dependencies on the targets in
# about.toml, as cargo-about sees them (no dev-dependencies, no build scripts).
target_args=()
while IFS= read -r t; do target_args+=(--target "$t"); done < <(
  python3 -I -c 'import sys, tomllib; print("\n".join(tomllib.load(open(sys.argv[1], "rb"))["targets"]))' \
    scripts/ci/about.toml
)
cargo tree --frozen --manifest-path "$manifest" -e normal "${target_args[@]}" \
  --prefix none -f '{p}' > "$shipped"

# MPL-2.0: only deny.toml's exceptions, each from the crates.io registry;
# about.toml names exactly the shipped ones.
cargo metadata --format-version 1 --frozen --manifest-path "$manifest" |
  python3 -I scripts/ci/check-mpl.py scripts/ci/about.toml deny.toml "$shipped"
cargo about generate --frozen --fail \
  --manifest-path "$manifest" \
  -c scripts/ci/about.toml scripts/ci/about.hbs -o "$tmp"

# Files in a bundle directory that are copied (licences and notices) and
# those that are the bundle itself; anything else stops the build.
is_notice() {
  case "$1" in
    LICENSE* | LICENCE* | NOTICE* | COPYING* | THIRD_PARTY* | AUTHORS* | PATENTS*) return 0 ;;
  esac
  return 1
}
is_bundle() {
  case "$1" in
    *.js | *.mjs | *.cjs) return 0 ;;
  esac
  return 1
}

{
  cat "$tmp"
  for dir in crates/islands/bundles/*/; do
    [ -d "$dir" ] || continue
    name="$(basename "$dir")"
    while IFS= read -r -d '' f; do
      rel="${f#"$dir"}"
      base="$(basename "$f")"
      if is_notice "$base"; then
        printf '\n================================================================================\n'
        printf 'JavaScript library built in: %s (%s)\n\n' "$name" "$rel"
        cat "$f"
      elif ! is_bundle "$base"; then
        echo "error: $f is neither JavaScript nor a licence or notice file;" \
          "name it LICENSE*/NOTICE*/THIRD_PARTY* or teach $0 about it" >&2
        exit 1
      fi
    done < <(find "$dir" -type f -print0 | LC_ALL=C sort -z)
  done
} > "$partial"

# MPL-2.0 (section 3.2) wants recipients told where the source is; the
# template says so whenever an MPL-2.0 crate is listed. Check it did.
if grep -q '(MPL-2.0)$' "$partial" && ! grep -q 'Source Code Form is available' "$partial"; then
  echo "MPL-2.0 crates are listed but the source-availability notice is missing" >&2
  exit 1
fi
chmod 644 "$partial"
mv -f "$partial" "$out"
echo "wrote $out ($(grep -c '' "$out") lines; $(grep -c '^JavaScript library built in' "$out") JS licence files)"
