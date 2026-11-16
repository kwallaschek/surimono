#!/usr/bin/env python3
"""Fails when a `cargo test` run ran fewer tests than it should (P2-PKG-10,
P2-PKG-11). Cargo passes a run whose name filter or cfg leaves 0 tests, which
is how #189 merged with `raikiri_build` at 0 tests and how a soak step whose
filter matches nothing would stay green. Reads the run's output (stdout and
stderr together, as `cargo test ... 2>&1 | tee LOG` writes it) and checks:

  --target NAME          the test target tests/NAME.rs ran (it has a
                         `Running tests/NAME.rs` section) and has at least one
                         test, passed or ignored; repeatable
  --gated-by FEATURE DIR every DIR/*.rs that mentions the feature in its code
                         is a --target (and there is at least one); see
                         feature_files for the rule
  --min-passed N         at least N tests passed (default 1), counted over the
                         targets named above, or over the whole run if none is
                         named

The per-target minimum is "passed or ignored", so a file whose tests are all
#[ignore]d (crates/dom/tests/layout_cost.rs) passes, while a file whose cfg
removed every test (#189's raikiri_build at 0) fails, whatever the other
files ran.

The counts come from the log's `Running` and `test result:` lines, so a test
that prints such a line could change them; only our own tests write to this
log, and none does.

Usage: count-tests.py LOG [--target NAME]... [--gated-by FEATURE DIR]...
                          [--min-passed N]
Exit status 0 when every check holds, 1 when one fails, 2 on bad usage. Called
by counted_cargo_test in scripts/ci/lib.sh; scripts/ci/test_ci_feature_steps.py
imports feature_files from here."""

import argparse
import glob
import os
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
RUNNING = re.compile(r"^\s*Running (unittests )?(\S+)(\s+\(|\s*$)")
DOCTESTS = re.compile(r"^\s*Doc-tests (\S+)")
RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored")


def sections(text):
    """{section: (passed, ignored)} for a cargo test log. A section is
    `tests/<x>.rs`, `unittests <path>` or `doc-tests <crate>`; a section that
    occurs more than once (tests/soak.rs in two crates) sums its runs."""
    out, current = {}, None
    for raw in text.splitlines():
        line = ANSI.sub("", raw).rstrip()
        m = RUNNING.match(line)
        if m:
            current = ("unittests " if m.group(1) else "") + m.group(2)
            out.setdefault(current, (0, 0))
            continue
        m = DOCTESTS.match(line)
        if m:
            current = "doc-tests " + m.group(1)
            out.setdefault(current, (0, 0))
            continue
        m = RESULT.match(line)
        if m and current is not None:
            passed, ignored = out[current]
            out[current] = (passed + int(m.group(1)), ignored + int(m.group(3)))
    return out


def mentions_feature(source, feature):
    """Whether Rust `source` names `feature = "<feature>"` outside `//`
    comments: in `#![cfg(...)]`, `#[cfg(...)]` on some items, `cfg(all(...))`,
    `cfg(not(...))` or `cfg_attr(...)` alike."""
    pattern = re.compile(r'\bfeature\s*=\s*"' + re.escape(feature) + r'"')
    code = "\n".join(l for l in source.splitlines() if not l.lstrip().startswith("//"))
    return bool(pattern.search(code))


def feature_files(feature, directory):
    """The rule for --gated-by: the stems of DIR/*.rs that mention the feature
    in their code (mentions_feature), whether the whole file is gated or only
    some items. The feature run must run each of them and find at least one
    test in it. For a file gated as a whole that proves the feature reached
    it; for a partly gated file (layout_real.rs) it only proves the file ran,
    since its ungated tests count too: one run can't tell which tests the
    cfg added. The rule fails closed: a file that names the feature only to
    turn something off (`cfg(not(...))`) is required as well."""
    names = []
    for path in sorted(glob.glob(os.path.join(directory, "*.rs"))):
        with open(path, encoding="utf-8") as f:
            if mentions_feature(f.read(), feature):
                names.append(os.path.splitext(os.path.basename(path))[0])
    return names


def check(text, targets, min_passed):
    """The list of failed checks, empty when all hold, and the passed count."""
    found = sections(text)
    problems = []
    for name in targets:
        key = f"tests/{name}.rs"
        if key not in found:
            problems.append(f"test target {key} did not run")
        elif sum(found[key]) == 0:
            problems.append(f"test target {key} ran 0 tests (none passed or ignored)")
    if targets:
        passed = sum(found.get(f"tests/{name}.rs", (0, 0))[0] for name in targets)
        scope = "in " + ", ".join(f"tests/{n}.rs" for n in targets)
    else:
        passed = sum(p for p, _ in found.values())
        scope = "in the whole run"
    if passed < min_passed:
        problems.append(f"{passed} tests passed {scope}, expected at least {min_passed}")
    return problems, passed


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("log")
    p.add_argument("--target", action="append", default=[])
    p.add_argument("--gated-by", nargs=2, action="append", default=[], metavar=("FEATURE", "DIR"))
    p.add_argument("--min-passed", type=int, default=1)
    a = p.parse_args(argv)
    targets = list(a.target)
    for feature, directory in a.gated_by:
        names = feature_files(feature, directory)
        if not names:
            print(f"count-tests: no test file in {directory} mentions feature {feature!r}",
                  file=sys.stderr)
            return 1
        targets += [n for n in names if n not in targets]
    with open(a.log, encoding="utf-8", errors="replace") as f:
        text = f.read()
    problems, passed = check(text, targets, a.min_passed)
    for msg in problems:
        print(f"count-tests: {msg}", file=sys.stderr)
    if problems:
        return 1
    print(f"count-tests: {passed} tests passed (at least {a.min_passed} expected)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
