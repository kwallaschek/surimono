#!/usr/bin/env bash
# Refreshes idl/*.idl from w3c/webref's ed/idl/ at a given commit. Run by hand
# only (it uses the network); builds and tests never call it.
#
# Usage: crates/webidl/fetch-webref.sh <webref commit sha>
# Then: set WEBREF_COMMIT in src/vendored.rs, update idl/SOURCES.md (commit,
# checksums, any new file and its licence), run
# `cargo run -p surimono-webidl --example regen`, and review the diffs.
set -euo pipefail
commit="${1:?usage: fetch-webref.sh <webref commit sha>}"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { echo "need a full 40-character commit sha" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git -C "$tmp" init -q
git -C "$tmp" remote add origin https://github.com/w3c/webref
git -C "$tmp" fetch -q --depth 1 origin "$commit"
git -C "$tmp" checkout -q FETCH_HEAD
for f in "$here"/idl/*.idl; do
  name="$(basename "$f")"
  if [ -f "$tmp/ed/idl/$name" ]; then
    cp "$tmp/ed/idl/$name" "$f"
  else
    echo "warning: $name is gone from webref at $commit; kept the old copy" >&2
  fi
done
cp "$tmp/LICENSE" "$here/idl/LICENSE-webref-MIT"
(cd "$here/idl" && sha256sum ./*.idl)
echo "copied ed/idl from w3c/webref $commit; now update src/vendored.rs and idl/SOURCES.md"
