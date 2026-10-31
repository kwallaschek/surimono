#!/usr/bin/env python3
"""Checks manifest.toml: every case is well-formed, every listed file exists,
and every .js file in this folder belongs to a case.

Usage: python3 -I fixtures/hostile/verify.py
"""

import pathlib
import sys
import tomllib

HERE = pathlib.Path(__file__).resolve().parent
OUTCOMES = {"MemoryLimit", "TimeLimit", "StackOverflow", "Exception", "Ok"}


def check_expectation(where: str, spec: dict) -> list[str]:
    errors = []
    has_one = "expect" in spec
    has_any = "expect_any" in spec
    if has_one == has_any:
        errors.append(f"{where}: needs exactly one of expect or expect_any")
        return errors
    outcomes = [spec["expect"]] if has_one else spec["expect_any"]
    bad = [o for o in outcomes if o not in OUTCOMES]
    if bad:
        errors.append(f"{where}: unknown outcome {bad}")
    if "Ok" in outcomes and not isinstance(spec.get("output"), str):
        errors.append(f"{where}: Ok is accepted but there is no output string")
    if "Ok" not in outcomes and "output" in spec:
        errors.append(f"{where}: output given but Ok is not accepted")
    if "error" in spec and "Exception" not in outcomes:
        errors.append(f"{where}: error given but Exception is not accepted")
    if not (HERE / spec.get("file", "")).is_file():
        errors.append(f"{where}: file {spec.get('file')!r} missing")
    return errors


def main() -> int:
    manifest = tomllib.loads((HERE / "manifest.toml").read_text(encoding="utf-8"))
    limits = manifest["limits"]
    errors = []
    for key in ("memory_bytes", "time_ms", "stack_bytes", "max_wall_ms"):
        if not isinstance(limits.get(key), int) or limits[key] <= 0:
            errors.append(f"limits.{key} must be a positive integer")
    ids = set()
    used = set()
    for case in manifest["case"]:
        cid = case.get("id", "?")
        if cid in ids:
            errors.append(f"{cid}: duplicate id")
        ids.add(cid)
        if not case.get("about"):
            errors.append(f"{cid}: needs about")
        if "steps" in case:
            if "file" in case:
                errors.append(f"{cid}: steps and file are exclusive")
            for i, step in enumerate(case["steps"], 1):
                errors += check_expectation(f"{cid} step {i}", step)
                used.add(step.get("file"))
        else:
            errors += check_expectation(cid, case)
            used.add(case.get("file"))
    for js in sorted(HERE.glob("*.js")):
        if js.name not in used:
            errors.append(f"{js.name}: not listed in manifest.toml")
    for e in errors:
        print(e)
    print(f"{len(manifest['case'])} cases, {len(used)} files: {'ok' if not errors else f'{len(errors)} problems'}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
