#!/usr/bin/env python3
"""Ranks the DOM interfaces by how many WPT tests they unblock.

Reads wpt/manifest.toml and the vendored tests, and the IDL in crates/webidl/idl.
A test needs an interface when its script names the interface, or a member only
that interface family declares. Members many interfaces share by name (`length`,
`item`) don't count, so the ranking errs towards too few needs: it orders work,
it isn't a result. Subtests are counted from the source (`test(`, `async_test(`,
`promise_test(`), so generated subtests in loops count once.

The greedy order answers "what next": at each step, the interface that completes
the most subtests given the interfaces before it. A test that needs a browser
feature surimono won't have (iframes, new windows, workers) is listed apart.

Usage: python3 -I wpt/tools/api_needs.py [--idl DIR] [--tests DIR] [--manifest FILE] [--json FILE]
"""

import argparse
import json
import pathlib
import re
import sys
import tomllib
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parents[2]
SUBTEST_RE = re.compile(r"(?<![\w.])(?:test|async_test|promise_test)\s*\(")
SCRIPT_RE = re.compile(r"<script\b(?![^>]*\bsrc\s*=)[^>]*>(.*?)</script\s*>", re.I | re.S)
INTERFACE_RE = re.compile(r"(?:partial\s+)?interface\s+(?:mixin\s+)?(\w+)\s*(?::\s*\w+\s*)?\{(.*?)\n?\};", re.S)
MEMBER_RE = re.compile(r"(?:^|\s)(\w+)\s*(?:\(|;|=)", re.M)
IDENT_RE = re.compile(r"[A-Za-z_]\w*")
PROP_RE = re.compile(r"\.\s*([A-Za-z_]\w*)")
IDL_WORDS = {"attribute", "readonly", "const", "static", "getter", "setter", "deleter", "stringifier", "inherit", "undefined", "constructor"}
# Things a one-document runner can't offer, by the text that needs them.
BLOCKERS = {
    "iframe": re.compile(r"<iframe|createElement\(\s*[\"']iframe|contentWindow|contentDocument"),
    "new window": re.compile(r"window\.open\s*\("),
    "worker": re.compile(r"new\s+(?:Shared)?Worker\s*\(|fetch_tests_from_worker"),
    "xhr or fetch": re.compile(r"XMLHttpRequest|\bfetch\s*\("),
    "testdriver": re.compile(r"testdriver"),
}


def load_idl(idl_dir: pathlib.Path) -> tuple[set[str], dict[str, set[str]]]:
    """Interface names, and each member name's declaring interfaces."""
    names: set[str] = set()
    members: dict[str, set[str]] = {}
    for f in sorted(idl_dir.glob("*.idl")):
        text = re.sub(r"/\*.*?\*/|//[^\n]*|\[[^\]]*\]", " ", f.read_text(encoding="utf-8"), flags=re.S)
        for name, body in INTERFACE_RE.findall(text):
            names.add(name)
            for line in body.split(";"):
                line = re.sub(r"\(.*", "(", line, flags=re.S).strip()
                m = MEMBER_RE.findall(line + ";")
                if m and m[0] not in IDL_WORDS:
                    members.setdefault(m[0], set()).add(name)
    return names, members


def script_of(rel: str, text: str) -> str:
    if rel.endswith(".js"):
        return text
    return "\n".join(SCRIPT_RE.findall(text))


def needs_of(script: str, interfaces: set[str], members: dict[str, set[str]]) -> set[str]:
    found = {w for w in IDENT_RE.findall(script) if w in interfaces}
    for prop in set(PROP_RE.findall(script)):
        owners = members.get(prop, set())
        if len(owners) == 1:
            found |= owners
    return found


def analyse(tests: list[dict], root: pathlib.Path, interfaces: set[str], members: dict[str, set[str]]) -> dict:
    rows = []
    blocked: Counter[str] = Counter()
    for t in tests:
        text = (root / t["file"]).read_text(encoding="utf-8", errors="replace")
        script = script_of(t["file"], text)
        for dep in t.get("needs", []):
            p = root / dep
            if p.is_file() and p.suffix == ".js" and dep != "resources/testharness.js":
                script += "\n" + p.read_text(encoding="utf-8", errors="replace")
        block = sorted(k for k, r in BLOCKERS.items() if r.search(text))
        count = max(1, len(SUBTEST_RE.findall(script_of(t["file"], text))))
        for b in block:
            blocked[b] += count
        rows.append({"slice": t["slice"], "file": t["file"], "subtests": count, "needs": sorted(needs_of(script, interfaces, members)), "blocked_by": block})
    open_rows = [r for r in rows if not r["blocked_by"]]
    total = sum(r["subtests"] for r in open_rows)
    done: set[str] = set()
    order = []
    remaining = {n for r in open_rows for n in r["needs"]}
    reachable = sum(r["subtests"] for r in open_rows if not r["needs"])
    while remaining:
        def gain(name: str) -> tuple[int, int, str]:
            have = done | {name}
            full = sum(r["subtests"] for r in open_rows if name in r["needs"] and set(r["needs"]) <= have)
            touch = sum(r["subtests"] for r in open_rows if name in r["needs"])
            return (full, touch, name)

        best = max(sorted(remaining), key=gain)
        full, touch, _ = gain(best)
        done.add(best)
        remaining.discard(best)
        reachable += full
        order.append({"interface": best, "completes": full, "touches": touch, "reachable_after": reachable, "of": total})
    return {"tests": rows, "order": order, "blocked": dict(sorted(blocked.items())), "subtests": total}


def table(result: dict) -> str:
    lines = ["| Next interface | Subtests it completes | Subtests that use it | Reachable after it |", "|---|---|---|---|"]
    for o in result["order"]:
        lines.append(f"| {o['interface']} | {o['completes']} | {o['touches']} | {o['reachable_after']} of {o['of']} |")
    if result["blocked"]:
        lines += ["", "Set aside (browser features a one-document runner lacks): " + ", ".join(f"{k} {v}" for k, v in result["blocked"].items())]
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--idl", type=pathlib.Path, default=ROOT / "crates" / "webidl" / "idl")
    ap.add_argument("--tests", type=pathlib.Path, default=ROOT / "wpt" / "tests")
    ap.add_argument("--manifest", type=pathlib.Path, default=ROOT / "wpt" / "manifest.toml")
    ap.add_argument("--json", type=pathlib.Path)
    args = ap.parse_args(argv)
    if not args.manifest.exists():
        print("wpt/manifest.toml isn't there yet: run wpt/tools/vendor.py first", file=sys.stderr)
        return 1
    manifest = tomllib.loads(args.manifest.read_text(encoding="utf-8"))
    interfaces, members = load_idl(args.idl)
    result = analyse(manifest.get("test", []), args.tests, interfaces, members)
    if args.json:
        args.json.write_text(json.dumps(result, indent=1) + "\n", encoding="utf-8")
    print(table(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
