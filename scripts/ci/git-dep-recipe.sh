#!/usr/bin/env bash
# The Rust recipe from README.md, as a new project outside this repository would
# use it: a git dependency on surimono plus its own [patch.crates-io] entry for
# rquickjs-sys (DECISIONS 2026-11-01). The build uses no Cargo.lock of ours, so
# this also catches a dependency's new release that breaks the MSRV, and it runs
# on the MSRV toolchain with the rust-version the README says to set. It then
# checks the README's other claim: without the patch entry the program doesn't
# link, rather than running on an unpatched engine.
#
# The block comes from README.md (recipe_manifest.py), so the docs and this check
# can't drift apart. Only the URL changes: the repository is private and the CI
# checkout is shallow, so cargo fetches a one-commit copy of HEAD over file://.
# Needs the network for crates.io. Run by nightly-extra.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

channel="$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)"
[ -n "$channel" ] || { echo "no channel in rust-toolchain.toml" >&2; exit 1; }
# Outside the repository rust-toolchain.toml doesn't apply.
export RUSTUP_TOOLCHAIN="$channel"
msrv="${channel%.*}"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

step "git dependency recipe: one-commit copy of HEAD"
repo="$scratch/repo"
mkdir "$repo"
git archive HEAD | tar -x -C "$repo"
git -C "$repo" init --quiet --initial-branch=main
git -C "$repo" add --all --force
git -C "$repo" -c user.name=ci -c user.email=ci@localhost commit --quiet -m snapshot

# Writes a project in $1 from recipe_manifest.py's output (extra flags in $2...).
project() {
  local dir="$1"; shift
  mkdir -p "$dir/src"
  {
    printf '[package]\nname = "recipe-check"\nversion = "0.0.0"\nedition = "2024"\n'
    printf 'rust-version = "%s"\npublish = false\n\n' "$msrv"
    python3 -I scripts/ci/recipe_manifest.py README.md docs/guide/rust.md "file://$repo" "$@"
  } > "$dir/Cargo.toml"
  cat > "$dir/src/main.rs" <<'RS'
//! One island per built-in family, rendered through the patched engine.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let font_path = std::env::args().nth(1).ok_or("usage: recipe-check <font>")?;
    let renderer = surimono::Renderer::new(surimono::Options::new().font(std::fs::read(font_path)?))?;
    let builtins = surimono::builtin_renderers();
    if builtins.len() < 3 || surimono::precompiled_builtins() != builtins.len() {
        return Err(format!(
            "{} built-ins, {} precompiled",
            builtins.len(),
            surimono::precompiled_builtins()
        )
        .into());
    }
    let html = concat!(
        r#"<script type="application/vnd.surimono+json" data-renderer="echarts" data-width="320" data-height="200">"#,
        r#"{"xAxis":{"type":"category","data":["a","b"]},"yAxis":{"type":"value"},"series":[{"type":"bar","data":[1,2]}]}"#,
        r#"</script>"#,
        r#"<script type="application/vnd.surimono+json" data-renderer="vega-lite" data-width="320" data-height="200">"#,
        r#"{"data":{"values":[{"x":"a","y":1},{"x":"b","y":2}]},"mark":"bar","encoding":{"x":{"field":"x","type":"nominal"},"y":{"field":"y","type":"quantitative"}}}"#,
        r#"</script>"#,
    );
    let out = renderer.render_islands(html)?;
    let svgs = out.matches("<svg").count();
    if svgs < 2 {
        return Err(format!("{svgs} SVGs in the output, expected at least 2").into());
    }
    println!("recipe-check: ok, {svgs} SVGs");
    Ok(())
}
RS
}

font="$(pwd)/fixtures/fonts/NotoSans-Regular.ttf"

step "git dependency recipe: with the patch, a program builds and renders ($channel)"
project "$scratch/with-patch"
(cd "$scratch/with-patch" && cargo run --quiet -- "$font") | tee "$scratch/run.log"
grep -q '^recipe-check: ok' "$scratch/run.log"

step "git dependency recipe: without the patch, the build fails at link time"
project "$scratch/no-patch" --no-patch
if (cd "$scratch/no-patch" && cargo build --quiet) > "$scratch/no-patch.log" 2>&1; then
  echo "built without the [patch.crates-io] entry: it would run on an unpatched engine" >&2
  exit 1
fi
if ! grep -q 'surimono_quickjs_patch_level' "$scratch/no-patch.log"; then
  echo "the build without the patch failed for another reason:" >&2
  tail -n 30 "$scratch/no-patch.log" >&2
  exit 1
fi
echo "failed as the README says (undefined surimono_quickjs_patch_level)"
