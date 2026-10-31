"""Checks MPL-2.0 crates before THIRD_PARTY_LICENSES.txt is written.

Usage: cargo metadata --format-version 1 ... |
       python3 -I check-mpl.py about.toml deny.toml shipped.txt

shipped.txt lists the crates the shipped library links, one per line, name
first (`cargo tree -e normal --prefix none -f '{p}'` for each target in
about.toml; dev-dependencies and build scripts are not shipped).

MPL-2.0 is file-level copyleft: the notice says each MPL-2.0 crate is used
unmodified and its source is published on crates.io (MPL-2.0 section 3.2).
That holds only if every crate that can only be used under MPL-2.0:

- comes from the crates.io registry (a [patch] to a path or git source would
  mean modified files we would have to publish ourselves), and
- is one of deny.toml's MPL-2.0 exceptions.

scripts/ci/about.toml accepts MPL-2.0 per crate, for exactly the MPL-2.0-only
crates in shipped.txt: never a crate deny.toml doesn't allow, never a shipped
one missing (cargo-about would fail), never one that isn't shipped (cargo-about
would warn "config found for a crate not present in the graph", which then
always means something).

Fails with a message naming each problem.
"""

import json
import re
import sys

try:
    import tomllib
except ModuleNotFoundError:  # Python before 3.11
    sys.exit("check-mpl.py needs Python 3.11 or later (tomllib)")

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
MPL = "MPL-2.0"


def satisfiable_without_mpl(expr):
    """Whether an SPDX licence expression can be satisfied without MPL-2.0:
    `MIT OR MPL-2.0` can (take MIT), `MPL-2.0` and `(MIT OR Apache-2.0) AND
    MPL-2.0` can't. The old `/` separator means OR; AND binds tighter."""
    tokens = re.findall(r"\(|\)|[^\s()/]+|/", expr)
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else None

    def take():
        nonlocal pos
        pos += 1
        return tokens[pos - 1]

    def term():
        if peek() == "(":
            take()
            value = disjunction()
            if peek() == ")":
                take()
            return value
        licence = take() if peek() else ""
        if peek() == "WITH":
            take()
            take()
        return licence.rstrip("+") != MPL

    def conjunction():
        value = term()
        while peek() == "AND":
            take()
            value = term() and value
        return value

    def disjunction():
        value = conjunction()
        while peek() in ("OR", "/"):
            take()
            value = conjunction() or value
        return value

    return disjunction()


def mpl_only(expr):
    """Whether some part of a crate can only be used under MPL-2.0."""
    return MPL in expr and not satisfiable_without_mpl(expr)


def main():
    about_path, deny_path, shipped_path = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(about_path, "rb") as f:
        about = tomllib.load(f)
    with open(deny_path, "rb") as f:
        deny = tomllib.load(f)
    with open(shipped_path, encoding="utf-8") as f:
        shipped = {line.split()[0] for line in f if line.strip()}
    if not shipped:
        sys.exit(f"error: {shipped_path} lists no crates")
    problems = []

    if MPL in about.get("accepted", []):
        problems.append(
            f"{about_path}: MPL-2.0 is accepted for every crate; accept it only "
            "in per-crate tables ([name] accepted = [\"MPL-2.0\"])"
        )
    about_mpl = {
        name
        for name, table in about.items()
        if isinstance(table, dict) and MPL in table.get("accepted", [])
    }
    deny_mpl = {
        e["name"]
        for e in deny.get("licenses", {}).get("exceptions", [])
        if MPL in e.get("allow", [])
    }
    if about_mpl - deny_mpl:
        problems.append(
            f"{about_path} accepts MPL-2.0 for crates {deny_path} doesn't allow: "
            f"{sorted(about_mpl - deny_mpl)}"
        )

    meta = json.load(sys.stdin)
    seen = []
    shipped_mpl = set()
    for pkg in meta["packages"]:
        expr = pkg.get("license") or ""
        if MPL not in expr:
            continue
        seen.append(f"{pkg['name']} {pkg['version']}")
        if pkg.get("source") != CRATES_IO:
            problems.append(
                f"{pkg['name']} {pkg['version']} ({expr}) comes from "
                f"{pkg.get('source') or 'a local path'}, not unmodified from crates.io; "
                "the MPL-2.0 notice would be false (publish the modified source or drop the patch)"
            )
        if not mpl_only(expr):
            continue
        if pkg["name"] not in deny_mpl:
            problems.append(
                f"{pkg['name']} {pkg['version']} is MPL-2.0 but not one of the crates "
                f"allowed in {deny_path}"
            )
        if pkg["name"] in shipped:
            shipped_mpl.add(pkg["name"])

    if shipped_mpl - about_mpl:
        problems.append(
            f"shipped MPL-2.0 crates missing from {about_path}: "
            f"{sorted(shipped_mpl - about_mpl)} (add [name] accepted = [\"MPL-2.0\"])"
        )
    if about_mpl - shipped_mpl:
        problems.append(
            f"{about_path} accepts MPL-2.0 for crates the shipped library doesn't link: "
            f"{sorted(about_mpl - shipped_mpl)} (remove them; deny.toml covers dev-dependencies)"
        )

    if problems:
        for p in problems:
            print(f"error: {p}", file=sys.stderr)
        sys.exit(1)
    print(
        f"MPL-2.0: {len(seen)} crate(s) in the tree, all unmodified from crates.io "
        f"and allowed: {', '.join(sorted(seen)) or 'none'}; shipped: "
        f"{', '.join(sorted(shipped_mpl)) or 'none'}",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
