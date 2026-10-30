#!/usr/bin/env python3
"""Check every font listed in fonts.toml against its pinned size and sha256.

Usage: python3 fixtures/fonts/verify.py   (exit 0 when all match)
"""

import hashlib
import pathlib
import sys
import tomllib

HERE = pathlib.Path(__file__).resolve().parent


def main() -> int:
    manifest = tomllib.loads((HERE / "fonts.toml").read_text(encoding="utf-8"))
    fonts = manifest.get("font", [])
    if not fonts:
        print("fonts.toml lists no fonts", file=sys.stderr)
        return 1
    failures = 0
    listed = set()
    for entry in fonts:
        path = HERE / entry["file"]
        listed.add(entry["file"])
        if not path.is_file():
            print(f"MISSING  {entry['file']}")
            failures += 1
            continue
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry["sha256"] or len(data) != entry["bytes"]:
            print(f"MISMATCH {entry['file']}: sha256 {digest}, {len(data)} bytes")
            failures += 1
        else:
            print(f"ok       {entry['file']}  {entry['family']} {entry['weight']}")
        if not (HERE / entry["license_file"]).is_file():
            print(f"MISSING  licence file {entry['license_file']}")
            failures += 1
    for path in sorted(HERE.iterdir()):
        if path.suffix.lower() in {".ttf", ".otf", ".ttc", ".woff", ".woff2"} and path.name not in listed:
            print(f"UNLISTED {path.name}: add it to fonts.toml")
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
