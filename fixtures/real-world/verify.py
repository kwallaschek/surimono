#!/usr/bin/env python3
"""Check the real-world suite manifest against the npm registry.

For every library in manifest.toml: download the tarball, check its sha512
integrity and sha256, and check that the dist file and licence files exist
in it with the pinned sha256 and size. When an entry records what
crates/islands embeds ([library.embedded]), check that file's sha256 and size
too; a recorded file that is missing is an error. Needs network access to registry.npmjs.org;
nothing is written outside a temporary directory.

Files vendored for page scripts ([[library.vendored]]) are checked in both modes.

--embedded-only checks only the embedded and vendored files, offline: what PR CI runs, so a
rebuilt bundle that no longer matches the manifest fails there. --root checks
the embedded files of another checkout (a worktree of the renderer's branch).

--release also fails while a licence file is pending ([library.license_pending]),
so a pending file can't be forgotten: the pre-release checklist runs it.

Usage: python3 -I fixtures/real-world/verify.py [--cache DIR] [--embedded-only] [--release] [--root DIR] [name ...]
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
        for v in lib.get("vendored", []):
            if "written_here" in v or "source_tarball" in v or "source_url" in v:
                continue
            data = member(v["member"])
            if data is None:
                errors.append(f"vendored member {v['member']} is not in the tarball")
            elif hashlib.sha256(data).hexdigest() != v["sha256"]:
                errors.append(f"tarball's {v['member']} differs from the manifest")
    for v in lib.get("vendored", []):
        if "source_tarball" in v:
            errors += check_source_tarball(v, cache)
        if "source_url" in v and hashlib.sha256(fetch(v["source_url"], cache)).hexdigest() != v["sha256"]:
            errors.append(f"{v['source_url']} differs from the manifest")
    return errors


def check_source_tarball(v: dict, cache: pathlib.Path | None) -> list[str]:
    """A vendored file that comes from another package's tarball."""
    data = fetch(v["source_tarball"], cache)
    if hashlib.sha256(data).hexdigest() != v["source_tarball_sha256"]:
        return [f"{v['source_tarball']} sha256 differs from the manifest"]
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        try:
            f = tar.extractfile(f"package/{v['member']}")
        except KeyError:
            f = None
        if f is None or hashlib.sha256(f.read()).hexdigest() != v["sha256"]:
            return [f"{v['member']} in {v['source_tarball']} differs from the manifest"]
    return []


def check_pending(lib: dict, root: pathlib.Path, release: bool) -> tuple[list[str], str]:
    """A licence file that isn't here yet. Tolerated with its task named, so the
    gap stays visible; --release fails on it. A file that has arrived must be a
    real MIT text, and is pinned by replacing this entry."""
    pending = lib.get("license_pending")
    if not pending:
        return [], ""
    for key in ("file", "source_url", "note", "task"):
        if key not in pending:
            return [f"license_pending.{key} missing"], ""
    if not (root / pending["note"]).is_file():
        return [f"licence note {pending['note']} is missing"], ""
    path = root / pending["file"]
    task = pending["task"]
    if not path.is_file():
        if release:
            return [f"licence file {pending['file']} is missing ({task}): the release can't go out without it"], ""
        return [], f"licence file pending ({task})"
    text = path.read_text(encoding="utf-8", errors="replace")
    if "MIT License" not in text and "The MIT License" not in text or "Permission is hereby granted" not in text:
        return [f"{pending['file']} isn't an MIT licence text"], ""
    if release:
        return [f"{pending['file']} is here but not pinned ({task}): replace license_pending with a vendored entry"], ""
    return [], f"licence file arrived: pin it and close {task}"


def check_vendored(lib: dict, root: pathlib.Path = ROOT) -> list[str]:
    """Offline: each file vendored for a page script matches the manifest, and
    the library keeps a licence file, or a note saying why it has none."""
    vendored = lib.get("vendored", [])
    errors = []
    for v in vendored:
        path = root / v["file"]
        if not path.is_file():
            errors.append(f"vendored {v['file']} is missing")
            continue
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != v["sha256"]:
            errors.append(f"vendored {v['file']} sha256 {hashlib.sha256(data).hexdigest()} differs from the manifest")
        if len(data) != v["bytes"]:
            errors.append(f"vendored {v['file']} is {len(data)} bytes")
    if vendored:
        files = {pathlib.PurePosixPath(v["file"]).name for v in vendored}
        paths = {v["file"] for v in vendored}
        for lf in lib["license_files"]:
            if pathlib.PurePosixPath(lf).name not in files:
                errors.append(f"licence file {lf} isn't vendored")
        for lf in lib.get("fonts_license_files", []):
            if not any(p.endswith(f"/{lf}") for p in paths):
                errors.append(f"fonts licence file {lf} isn't vendored")
        if not lib["license_files"] and not lib.get("license_pending") and not any(v.get("license") for v in vendored):
            errors.append("no licence file and no license_pending")
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
    ap.add_argument("--release", action="store_true", help="also fail on a licence file that is still pending")
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
        pending_errors, pending_note = check_pending(lib, args.root.resolve(), args.release)
        errors += emb_errors + check_vendored(lib, args.root.resolve()) + pending_errors
        note = "; ".join(x for x in (note, pending_note) if x)
        failed += bool(errors)
        status = "ok  " if not errors else "FAIL"
        print(f"{status} {lib['name']} {lib['version']} ({lib['status']}, {lib['license']}){f'; {note}' if note else ''}" + "".join(f"\n     {e}" for e in errors))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
