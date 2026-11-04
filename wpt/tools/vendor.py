#!/usr/bin/env python3
"""Vendors the WPT slices from a local web-platform-tests checkout.

Needs a checkout at exactly the commit pinned in wpt/slices.toml and uses no
network. It copies, keeping WPT's paths under wpt/tests/: LICENSE.md,
resources/testharness.js, every testharness test in a slice, and the files those
tests name (<script src>, // META: script=). Reftests, manual tests and files above
max_file_bytes stay out. A file named through a symlink, or outside the checkout,
is never copied. It writes wpt/manifest.toml: per slice, each test, the URLs it
runs at (wpt/README.md, "Test ids") and what it needs, plus sha256 and size of
every vendored file. A test with no window variant (`// META: global=` naming
only workers) is left out. Nothing in wpt/ changes until all of that has worked.

--check verifies the committed files against the manifest, offline, and that
the manifest is for the commit slices.toml pins.

Usage: python3 -I wpt/tools/vendor.py --from CHECKOUT
       python3 -I wpt/tools/vendor.py --check
"""

import argparse
import hashlib
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
ALWAYS = ["LICENSE.md", "resources/testharness.js"]
TEST_EXT = (".html", ".htm", ".xhtml", ".xht", ".svg")
GENERATED = (".any.js", ".window.js")
# testharnessreport.js is the embedder's hook; the runner supplies its own.
SUPPLIED = {"/resources/testharnessreport.js"}
SCRIPT_RE = re.compile(r"<script\b[^>]*\bsrc\s*=\s*[\"']?([^\"'\s>]+)", re.I)
META_RE = re.compile(r"^//\s*META:\s*script=(\S+)", re.M)
META_GLOBAL_RE = re.compile(r"^//\s*META:\s*global=(\S+)", re.M)
META_VARIANT_RE = re.compile(r"^//\s*META:\s*variant=(\S+)", re.M)
VARIANT_RE = re.compile(r"<meta\b[^>]*\bname\s*=\s*[\"']?variant[\"']?[^>]*\bcontent\s*=\s*[\"']([^\"']*)", re.I)


def is_test(rel: str, text: str) -> bool:
    name = rel.rsplit("/", 1)[-1]
    if name.startswith("support") or "/resources/" in f"/{rel}" or "/support/" in f"/{rel}":
        return False
    if name.endswith(GENERATED):
        return True
    if "-manual." in name or "-ref." in name:
        return False
    return name.endswith(TEST_EXT) and "testharness.js" in text


def dependencies(rel: str, text: str) -> list[str]:
    """Files a test names, as paths from the WPT root."""
    refs = META_RE.findall(text) if rel.endswith(GENERATED) else SCRIPT_RE.findall(text)
    out = []
    for ref in refs:
        ref = ref.split("?", 1)[0].split("#", 1)[0]
        if re.match(r"^[a-z]+:|^//", ref, re.I):
            continue
        path = pathlib.PurePosixPath(ref.lstrip("/") if ref.startswith("/") else f"{pathlib.PurePosixPath(rel).parent}/{ref}")
        parts: list[str] = []
        for p in path.parts:
            if p == "..":
                if not parts:
                    raise ValueError(f"{rel}: {ref} leaves the WPT root")
                parts.pop()
            elif p != ".":
                parts.append(p)
        normal = "/".join(parts)
        if f"/{normal}" not in SUPPLIED:
            out.append(normal)
    return sorted(set(out))


def urls(rel: str, text: str) -> list[str]:
    """The test ids a file runs as in a window, without the leading slash: empty
    for a generated test whose `// META: global=` has no window scope."""
    if rel.endswith(".any.js"):
        scopes: set[str] = set()
        for value in META_GLOBAL_RE.findall(text):
            scopes.update(value.split(","))
        if scopes and not scopes & {"window", "default"}:
            return []
        base = rel[: -len(".js")] + ".html"
    elif rel.endswith(".window.js"):
        base = rel[: -len(".js")] + ".html"
    else:
        base = rel
    variants = META_VARIANT_RE.findall(text) if rel.endswith(GENERATED) else VARIANT_RE.findall(text)
    return [base + v for v in sorted(set(variants))] or [base]


def safe_file(src: pathlib.Path, rel: str) -> pathlib.Path | None:
    """The file at rel, if it is a regular file reached through no symlink and
    inside the checkout. A checkout is third-party: a link in it must not put a
    local file into a commit."""
    path = src
    for part in pathlib.PurePosixPath(rel).parts:
        path = path / part
        if path.is_symlink():
            return None
    if not path.is_file() or not path.resolve().is_relative_to(src.resolve()):
        return None
    return path


def toml_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def vendor(src: pathlib.Path, root: pathlib.Path, slices: dict) -> None:
    head = subprocess.run(["git", "-C", str(src), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    if head != slices["commit"]:
        raise SystemExit(f"checkout is at {head}, slices.toml pins {slices['commit']}")
    limit = slices["max_file_bytes"]
    wanted: set[str] = set(ALWAYS)
    skipped: list[str] = []
    entries: list[tuple[str, str, list[str], list[str]]] = []
    for sl in slices["slice"]:
        for top in sl["paths"]:
            for f in sorted((src / top).rglob("*")):
                if not f.is_file() or f.is_symlink():
                    continue
                rel = f.relative_to(src).as_posix()
                data = f.read_bytes()
                text = data.decode("utf-8", "replace")
                if not is_test(rel, text):
                    continue
                if len(data) > limit:
                    skipped.append(rel)
                    continue
                ids = urls(rel, text)
                if not ids:
                    continue
                deps = dependencies(rel, text)
                entries.append((sl["id"], rel, ids, deps))
                wanted.add(rel)
                wanted.update(deps)
    for rel in ALWAYS:
        f = safe_file(src, rel)
        if f is None or f.stat().st_size > limit:
            raise SystemExit(f"{rel} can't be vendored: every test depends on it")
    lines = ["# Generated by wpt/tools/vendor.py; never edited by hand.", "schema = 1", f"commit = {toml_str(head)}", f"source = {toml_str(slices['source'])}", ""]
    for rel in sorted(skipped):
        lines += ["[[skipped]]", f"file = {toml_str(rel)}", 'reason = "above max_file_bytes"', ""]
    for sid, rel, ids, deps in entries:
        lines += ["[[test]]", f"slice = {toml_str(sid)}", f"file = {toml_str(rel)}", f"urls = [{', '.join(toml_str(u) for u in ids)}]", f"needs = [{', '.join(toml_str(d) for d in deps)}]", ""]
    copies: list[tuple[str, bytes]] = []
    for rel in sorted(wanted):
        f = safe_file(src, rel)
        if f is None or f.stat().st_size > limit:
            lines += ["[[missing]]", f"file = {toml_str(rel)}", ""]
            continue
        data = f.read_bytes()
        copies.append((rel, data))
        lines += ["[[file]]", f"path = {toml_str(rel)}", f'sha256 = "{hashlib.sha256(data).hexdigest()}"', f"bytes = {len(data)}", ""]
    dest = root / "tests"
    shutil.rmtree(dest, ignore_errors=True)
    for rel, data in copies:
        out = dest / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(data)
    (root / "manifest.toml").write_text("\n".join(lines), encoding="utf-8")


def check(root: pathlib.Path) -> list[str]:
    manifest = root / "manifest.toml"
    if not manifest.exists():
        # Not vendored yet: nothing to check, unless tests came without a manifest.
        return ["wpt/tests exists but manifest.toml doesn't"] if (root / "tests").exists() else []
    m = tomllib.loads(manifest.read_text(encoding="utf-8"))
    errors = []
    pinned = tomllib.loads((root / "slices.toml").read_text(encoding="utf-8"))["commit"]
    if m.get("commit") != pinned:
        errors.append(f"manifest is for {m.get('commit')}, slices.toml pins {pinned}")
    listed = set()
    for f in m.get("file", []):
        listed.add(f["path"])
        p = root / "tests" / f["path"]
        if not p.is_file():
            errors.append(f"{f['path']} is missing")
        elif hashlib.sha256(p.read_bytes()).hexdigest() != f["sha256"]:
            errors.append(f"{f['path']} differs from the manifest")
    tests = root / "tests"
    if tests.exists():
        for p in tests.rglob("*"):
            if p.is_file() and p.relative_to(tests).as_posix() not in listed:
                errors.append(f"{p.relative_to(tests).as_posix()} isn't in the manifest")
    return errors


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", type=pathlib.Path)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--root", type=pathlib.Path, default=ROOT, help=argparse.SUPPRESS)
    args = ap.parse_args(argv)
    if args.check:
        errors = check(args.root)
        print("\n".join(errors) or "wpt: vendored files match the manifest")
        return 1 if errors else 0
    if not args.src:
        ap.error("--from CHECKOUT or --check")
    vendor(args.src.resolve(), args.root, tomllib.loads((args.root / "slices.toml").read_text(encoding="utf-8")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
