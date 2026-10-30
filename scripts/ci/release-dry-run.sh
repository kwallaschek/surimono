#!/usr/bin/env bash
# Release dry-run: everything `cargo publish` and `gem push` would do, except
# publishing. Never publishes, never tags. Run on the release PR's branch
# (release/v*, where full.yml runs it through nightly-extra or by hand).
# Usage: scripts/ci/release-dry-run.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
out="${CI_JOB_OUT:-ci-out/release-dry-run}"
mkdir -p "$out"

# Crates published to crates.io, in dependency order; and those never published.
PUBLISHED=(surimono-core surimono-gfx surimono-islands surimono surimono-cli)
NEVER=(surimono-ruby)

step "no stand-in renderer left"
if git grep -n 'surimono-stub' -- crates bindings; then
  echo "surimono-stub is still in crates/ or bindings/" >&2; exit 1
fi

step "versions and publish flags"
cargo metadata --format-version 1 --no-deps --locked > "$out/metadata.json"
python3 -I - "$out/metadata.json" "$(ruby -e 'load "bindings/ruby/lib/surimono/version.rb"; print Surimono::VERSION')" \
  "${PUBLISHED[*]}" "${NEVER[*]}" <<'PY'
import json, sys
meta, gem_version, published, never = sys.argv[1], sys.argv[2], sys.argv[3].split(), sys.argv[4].split()
pkgs = {p["name"]: p for p in json.load(open(meta))["packages"]}
errors = []
for name in published:
    p = pkgs.get(name)
    if p is None:
        errors.append(f"{name}: not in the workspace"); continue
    if p["publish"] == []:
        errors.append(f"{name}: publish = false, but it is released")
    if p["version"] != gem_version:
        errors.append(f"{name}: version {p['version']}, the gem says {gem_version}")
    if p["version"] == "0.0.0":
        errors.append(f"{name}: still version 0.0.0")
    for d in p["dependencies"]:
        if d.get("path") and d["kind"] in (None, "build") and d["name"] not in published:
            errors.append(f"{name}: depends on unreleased {d['name']}")
for name in never:
    if pkgs.get(name, {}).get("publish") != []:
        errors.append(f"{name}: must say publish = false")
others = sorted(n for n, p in pkgs.items() if n not in published + never and p["publish"] != [])
for name in others:
    print(f"warning: {name} is publishable but not part of this release; set publish = false until it is")
for e in errors:
    print("error:", e)
sys.exit(1 if errors else 0)
PY

step "cargo package (all released crates at once, so they resolve each other)"
# Packaging crates that depend on not-yet-published workspace crates needs
# Cargo 1.90 or later (it resolves them against a local overlay); the pinned
# MSRV toolchain's Cargo can't, so this one step uses stable Cargo. Nothing is
# compiled here (--no-verify): the workspace build and tests on the MSRV cover
# the code, and surimono-core gets a verified dry-run below.
rustup toolchain list | grep -q '^stable' || rustup toolchain install stable --profile minimal
pkgs=(); for c in "${PUBLISHED[@]}"; do pkgs+=(-p "$c"); done
RUSTUP_TOOLCHAIN=stable cargo package --locked --no-verify --allow-dirty "${pkgs[@]}" 2>&1 | tee "$out/package.log"
for c in "${PUBLISHED[@]}"; do
  RUSTUP_TOOLCHAIN=stable cargo package --locked --list --allow-dirty -p "$c" > "$out/$c.files"
  f="$(ls "${CARGO_TARGET_DIR:-target}/package/$c-"[0-9]*.crate | tail -n1)"
  size=$(wc -c < "$f")
  echo "$c: $(wc -l < "$out/$c.files") files, $size bytes ($f)"
  [ "$size" -lt $((10 * 1024 * 1024)) ] || { echo "$c is over crates.io's 10 MB limit" >&2; exit 1; }
done

step "cargo publish --dry-run (first crate; the others need it on crates.io first)"
# Verified: built from its .crate alone, as crates.io users get it. This is
# where a dependency that only works through the workspace's [patch] (the
# vendored QuickJS) shows up: [patch] doesn't travel with a published crate.
cargo publish --dry-run --locked --allow-dirty -p "${PUBLISHED[0]}" 2>&1 | tee "$out/publish-dry-run.log"

step "source gem"
ruby_path
(cd bindings/ruby && bundle install --quiet && bundle exec rake gem:source) | tee "$out/gem.log"
gem_file="$(ls bindings/ruby/pkg/surimono-*.gem | grep -v -- '-x86\|-arm\|-aarch' | tail -n1)"
tar -xOf "$gem_file" data.tar.gz | tar -tzf - | sort > "$out/gem-contents.txt"
grep -qx THIRD_PARTY_LICENSES.txt "$out/gem-contents.txt" || { echo "the gem has no THIRD_PARTY_LICENSES.txt" >&2; exit 1; }
echo "built $gem_file ($(wc -l < "$out/gem-contents.txt") files)"
step "ok: nothing was published"
