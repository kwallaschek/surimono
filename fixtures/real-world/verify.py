#!/usr/bin/env python3
"""Check the real-world suite manifest against the npm registry.

For every library in manifest.toml: download the tarball, check its sha512
integrity and sha256, and check that the bundle file and licence files exist
in it with the pinned sha256 and size. Needs network access to
registry.npmjs.org; nothing is written outside a temporary directory.

Usage: python3 -I fixtures/real-world/verify.py [--cache DIR] [name ...]
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
        bundle = member(lib["bundle_file"])
        if bundle is None:
            errors.append(f"bundle {lib['bundle_file']} missing")
        else:
            if hashlib.sha256(bundle).hexdigest() != lib["bundle_sha256"]:
                errors.append(f"bundle sha256 {hashlib.sha256(bundle).hexdigest()} differs")
            if len(bundle) != lib["bundle_bytes"]:
                errors.append(f"bundle is {len(bundle)} bytes")
        for lf in lib["license_files"]:
            if member(lf) is None:
                errors.append(f"licence file {lf} missing")
    return errors


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=pathlib.Path, help="keep downloaded tarballs here")
    ap.add_argument("names", nargs="*")
    args = ap.parse_args()
    manifest = tomllib.loads((HERE / "manifest.toml").read_text(encoding="utf-8"))
    libs = [lib for lib in manifest["library"] if not args.names or lib["name"] in args.names]
    failed = 0
    for lib in libs:
        errors = check(lib, args.cache)
        failed += bool(errors)
        status = "ok  " if not errors else "FAIL"
        print(f"{status} {lib['name']} {lib['version']} ({lib['status']}, {lib['license']})" + "".join(f"\n     {e}" for e in errors))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
