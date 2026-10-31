#!/usr/bin/env python3
"""Check the real-world suite manifest against the npm registry.

For every library in manifest.toml: download the tarball, check its sha512
integrity and sha256, and check that the dist file and licence files exist
in it with the pinned sha256 and size. When an entry records what
crates/islands embeds ([library.embedded]), check that file's sha256 and size
too; a recorded file that is missing is an error. Needs network access to registry.npmjs.org;
nothing is written outside a temporary directory.

--embedded-only checks only the embedded files, offline: what PR CI runs, so a
rebuilt bundle that no longer matches the manifest fails there. --root checks
the embedded files of another checkout (a worktree of the renderer's branch).

Usage: python3 -I fixtures/real-world/verify.py [--cache DIR] [--embedded-only] [--root DIR] [name ...]
"""

import argparse
import base64
import hashlib
import io
import pathlib
import sys
import tarfile
import tomllib
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def fetch(url: str, cache: pathlib.Path | None) -> bytes:
    if cache:
        cached = cache / url.rsplit("/", 1)[-1]
        if cached.is_file():
            return cached.read_bytes()
    with urllib.request.urlopen(url, timeout=120) as r:  # noqa: S310 (pinned https URL)
        data = r.read()
    if cache:
        cache.mkdir(parents=True, exist_ok=True)
        (cache / url.rsplit("/", 1)[-1]).write_bytes(data)
    return data


def check(lib: dict, cache: pathlib.Path | None) -> list[str]:
    errors = []
    data = fetch(lib["npm_tarball"], cache)
    algo, _, want = lib["integrity"].partition("-")
    got = base64.b64encode(hashlib.new(algo, data).digest()).decode()
    if got != want:
        errors.append(f"integrity {algo}-{got} != manifest")
    if hashlib.sha256(data).hexdigest() != lib["tarball_sha256"]:
        errors.append("tarball sha256 differs")
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        names = set(tar.getnames())

        def member(rel: str) -> bytes | None:
            name = f"package/{rel}"
            if name not in names:
                return None
            f = tar.extractfile(name)
            return f.read() if f else None

        pkg = member("package.json")
        if pkg is None or f'"version": "{lib["version"]}"' not in pkg.decode():
            errors.append("package.json version differs")
        dist = member(lib["dist_file"])
        if dist is None:
            errors.append(f"dist file {lib['dist_file']} missing")
        else:
            if hashlib.sha256(dist).hexdigest() != lib["dist_sha256"]:
                errors.append(f"dist file sha256 {hashlib.sha256(dist).hexdigest()} differs")
            if len(dist) != lib["dist_bytes"]:
                errors.append(f"dist file is {len(dist)} bytes")
        for lf in lib["license_files"]:
            if member(lf) is None:
                errors.append(f"licence file {lf} missing")
    return errors


def check_embedded(lib: dict, root: pathlib.Path = ROOT) -> tuple[list[str], str]:
    """Checks [library.embedded] against the file under root, if both exist."""
    emb = lib.get("embedded")
    if not emb:
        return [], "embedded build not recorded yet"
    for key in ("file", "sha256", "bytes", "built_by", "built_from", "license_files", "recorded_from"):
        if key not in emb:
            return [f"embedded.{key} missing"], ""
    path = root / emb["file"]
    if not path.is_file():
        # The bundles are on main: a missing one means it moved or was dropped
        # without the manifest following.
        return [f"embedded {emb['file']} is missing (recorded from {emb['recorded_from']})"], ""
    data = path.read_bytes()
    errors = []
    if hashlib.sha256(data).hexdigest() != emb["sha256"]:
        errors.append(f"embedded sha256 {hashlib.sha256(data).hexdigest()} differs from the manifest")
    if len(data) != emb["bytes"]:
        errors.append(f"embedded file is {len(data)} bytes")
    for lf in emb["license_files"]:
        if not (path.parent / lf).is_file():
            errors.append(f"embedded licence file {lf} missing next to {emb['file']}")
    return errors, f"embedded {emb['file']} checked"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=pathlib.Path, help="keep downloaded tarballs here")
    ap.add_argument("--embedded-only", action="store_true", help="check only the embedded files (offline)")
    ap.add_argument("--root", type=pathlib.Path, default=ROOT, help="checkout whose embedded files are checked (default: this one)")
    ap.add_argument("--manifest", type=pathlib.Path, default=HERE / "manifest.toml", help=argparse.SUPPRESS)
    ap.add_argument("names", nargs="*")
    args = ap.parse_args(argv)
    manifest = tomllib.loads(args.manifest.read_text(encoding="utf-8"))
    libs = [lib for lib in manifest["library"] if not args.names or lib["name"] in args.names]
    unknown = set(args.names) - {lib["name"] for lib in manifest["library"]}
    if unknown:
        print(f"FAIL unknown libraries: {', '.join(sorted(unknown))}")
        return 1
    failed = 0
    for lib in libs:
        errors = [] if args.embedded_only else check(lib, args.cache)
        emb_errors, note = check_embedded(lib, args.root.resolve())
        errors += emb_errors
        failed += bool(errors)
        status = "ok  " if not errors else "FAIL"
        print(f"{status} {lib['name']} {lib['version']} ({lib['status']}, {lib['license']}){f'; {note}' if note else ''}" + "".join(f"\n     {e}" for e in errors))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
