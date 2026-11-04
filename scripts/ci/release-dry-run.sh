#!/usr/bin/env bash
# Release dry-run: everything a release does, except publishing. Never
# publishes, never tags. Run on the release PR's branch (release/v*, where
# full.yml runs it through nightly-extra, or by hand).
#
# v0.1's release artifacts are the Ruby gem and the CLI built from a clone of
# the repository (Kay, checkpoint 2). No crate goes to crates.io: [patch]
# doesn't travel with a published crate, so its users would link the
# unpatched rquickjs-sys. This script checks that no crate is publishable and
# that every version and the CHANGELOG agree, builds and runs the CLI the way
# README.md tells users to, and builds the source gem with the patched engine
# in it. full.yml's ruby jobs install and test the source gem on Ruby 3.2 to
# 4.0, and its native-gems jobs build and check the precompiled gems.
# Usage: scripts/ci/release-dry-run.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
out="${CI_JOB_OUT:-ci-out/release-dry-run}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# Release blockers found below; reported together at the end, so one run
# shows all of them.
blockers=()

step "no stand-in renderer left"
if git grep -n 'surimono-stub' -- crates bindings; then
  echo "surimono-stub is still in crates/ or bindings/" >&2; exit 1
fi

version="$(ruby -e 'load "bindings/ruby/lib/surimono/version.rb"; print Surimono::VERSION')"
echo "release version (the gem's VERSION): $version"

step "no crate is publishable; every shipped crate is at $version"
cargo metadata --format-version 1 --no-deps --locked > "$out/metadata.json"
if ! python3 -I - "$out/metadata.json" "$version" "$REPO_ROOT" <<'PY'
import json, os, sys
meta, version, root = sys.argv[1], sys.argv[2], sys.argv[3]
errors = []
for p in sorted(json.load(open(meta))["packages"], key=lambda p: p["name"]):
    name = p["name"]
    rel = os.path.relpath(p["manifest_path"], root)
    # cargo metadata says [] for publish = false, null for publishable.
    if p["publish"] != []:
        errors.append(f"{name} is publishable; v0.1 publishes no crate (Kay, checkpoint 2): "
                      "set publish = false")
    # The crates the gem and the CLI are built from share the gem's version;
    # examples/ (the docs' runnable examples) aren't shipped and stay 0.0.0.
    if rel.split(os.sep)[0] in ("crates", "bindings") and p["version"] != version:
        errors.append(f"{name}: version {p['version']}, the gem says {version}")
    print(f"{name} {p['version']} publish={'false' if p['publish'] == [] else p['publish']}")
for e in errors:
    print("error:", e)
sys.exit(1 if errors else 0)
PY
then
  blockers+=("crate versions or publish flags (see above)")
fi
[ "$version" != 0.0.0 ] || blockers+=("the version is still 0.0.0")

step "CHANGELOG.md has a dated entry for $version"
heading="$(grep -m1 -E "^## \[$version\]" CHANGELOG.md || true)"
echo "${heading:-(no heading for $version)}"
if [ -z "$heading" ]; then
  blockers+=("CHANGELOG.md has no '## [$version]' entry")
elif ! printf '%s\n' "$heading" | grep -Eq "^## \[$version\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$"; then
  blockers+=("CHANGELOG.md: '$heading' has no release date (YYYY-MM-DD)")
fi
grep -q "^\[$version\]: .*/releases/tag/v$version\$" CHANGELOG.md \
  || blockers+=("CHANGELOG.md has no [$version] link to the v$version tag")

step "CLI: cargo install --locked --path crates/cli (as README.md tells users)"
# A clone has the workspace's [patch], so the CLI links the patched engine;
# surimono-core refuses to link without its marker symbol (DECISIONS, #32).
# Release profile, as users get it; the shared target dir keeps it cached.
if cargo install --locked --path crates/cli --root "$scratch/cli" > "$out/cli-install.log" 2>&1; then
  tail -n 2 "$out/cli-install.log"
  cli="$scratch/cli/bin/surimono"
  got="$("$cli" --version)"
  echo "$got"
  [ "$got" = "surimono $version" ] || blockers+=("the CLI says '$got', expected 'surimono $version'")
  # One island per built-in renderer family, rendered with the fixtures' font.
  cat > "$scratch/in.html" <<'HTML'
<!doctype html><html><body>
<script type="application/vnd.surimono+json" data-renderer="echarts" data-width="320" data-height="200">
{"xAxis":{"type":"category","data":["a","b"]},"yAxis":{"type":"value"},"series":[{"type":"bar","data":[1,2]}]}
</script>
<script type="application/vnd.surimono+json" data-renderer="vega-lite" data-width="320" data-height="200">
{"data":{"values":[{"x":"a","y":1},{"x":"b","y":2}]},"mark":"bar","encoding":{"x":{"field":"x","type":"nominal"},"y":{"field":"y","type":"quantitative"}}}
</script>
</body></html>
HTML
  if "$cli" render "$scratch/in.html" -o "$out/cli-render.html" \
      --font fixtures/fonts/NotoSans-Regular.ttf > "$out/cli-render.log" 2>&1; then
    islands="$(grep -o 'class="surimono-island' "$out/cli-render.html" | wc -l)"
    svgs="$(grep -o '<svg' "$out/cli-render.html" | wc -l)"
    echo "surimono render: exit 0, $islands islands, $svgs <svg> elements"
    [ "$islands" -eq 2 ] && [ "$svgs" -ge 2 ] \
      || blockers+=("the CLI rendered $islands islands and $svgs SVGs, expected 2 and at least 2")
  else
    cat "$out/cli-render.log"
    blockers+=("surimono render failed on a two-island document (see cli-render.log)")
  fi
else
  grep -E 'undefined reference|error(\[|:)' "$out/cli-install.log" | head -n 5 || tail -n 20 "$out/cli-install.log"
  blockers+=("cargo install --locked --path crates/cli failed (see cli-install.log)")
fi

step "source gem"
ruby_path
(cd bindings/ruby && bundle install --quiet && bundle exec rake gem:source) | tee "$out/gem.log"
gem_file="$(ls bindings/ruby/pkg/surimono-"$version".gem)"
tar -xOf "$gem_file" data.tar.gz | tar -tzf - | sort > "$out/gem-contents.txt"
echo "built $gem_file ($(wc -l < "$out/gem-contents.txt") files)"
grep -qx THIRD_PARTY_LICENSES.txt "$out/gem-contents.txt" \
  || blockers+=("the gem has no THIRD_PARTY_LICENSES.txt")
# The patched engine: the vendored rquickjs-sys and the [patch] that selects it.
tar -xOf "$gem_file" data.tar.gz | tar -xzOf - Cargo.toml > "$out/gem-Cargo.toml"
if grep -qx 'vendor/rquickjs-sys/Cargo.toml' "$out/gem-contents.txt" \
    && grep -Eq '^rquickjs-sys *= *\{ *path *= *"vendor/rquickjs-sys"' "$out/gem-Cargo.toml"; then
  echo "the gem carries vendor/rquickjs-sys and its [patch.crates-io] entry"
else
  blockers+=("the gem doesn't carry the patched engine (vendor/rquickjs-sys plus its [patch] entry)")
fi

if [ ${#blockers[@]} -gt 0 ]; then
  step "release blockers (nothing was published)"
  printf 'error: %s\n' "${blockers[@]}" >&2
  exit 1
fi
step "ok: nothing was published"
