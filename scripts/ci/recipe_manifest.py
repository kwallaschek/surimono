#!/usr/bin/env python3
"""Prints the [dependencies]/[patch.crates-io] block the docs tell Rust users to
copy, pointed at another repository URL, for scripts/ci/git-dep-recipe.sh.

Usage: recipe_manifest.py <README.md> <guide.md> <repository url> [--no-patch]
Exits 1 when the README has no such block, or the guide's two lines differ from
the README's, so a docs change can't quietly stop the nightly check meaning
anything.
"""
import re
import sys

REPO = "https://github.com/kwallaschek/surimono"


def recipe_block(markdown):
    """The first ```toml block with both the dependency and the patch."""
    for block in re.findall(r"^```toml\n(.*?)^```$", markdown, re.M | re.S):
        if "[patch.crates-io]" in block and re.search(r"^surimono = ", block, re.M):
            if re.search(r"^rquickjs-sys = ", block, re.M):
                return block
    raise ValueError("no ```toml block with surimono and a [patch.crates-io] rquickjs-sys")


def recipe_lines(block):
    return [line for line in block.splitlines() if line.startswith(("surimono = ", "rquickjs-sys = "))]


def without_patch(block):
    return block.split("[patch.crates-io]")[0].rstrip() + "\n"


def main(argv):
    readme, guide, url = argv[1:4]
    no_patch = "--no-patch" in argv[4:]
    try:
        block = recipe_block(open(readme, encoding="utf-8").read())
        other = recipe_block(open(guide, encoding="utf-8").read())
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    if recipe_lines(block) != recipe_lines(other):
        print(f"error: {readme} and {guide} give different recipes", file=sys.stderr)
        return 1
    if REPO not in block:
        print(f"error: the recipe doesn't name {REPO}", file=sys.stderr)
        return 1
    if no_patch:
        block = without_patch(block)
    sys.stdout.write(block.replace(REPO, url))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
