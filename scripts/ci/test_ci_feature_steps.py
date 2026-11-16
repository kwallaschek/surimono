"""Tests that scripts/ci.sh always checks surimono-dom's `raikiri` feature
(P2-PKG-09, P2-PKG-10) and that scripts/ci/soak.sh runs the soaks, the
Raikiri layout soak among them (P2-PKG-11). The feature is off by default, so
without these steps CI builds none of the code behind it: clippy never reads it
and crates/dom/tests/raikiri_build.rs compiles to 0 tests, which is how #189
merged.

The behaviour tests run the real scripts, copied into a scratch tree, with a
stub `cargo` (and stub python3 -m unittest, bundle, ruby, rustup, cargo-deny,
and the sub-scripts ci.sh calls) first on PATH. The stub records every call and
prints the libtest output a scenario asks for, so the tests see which cargo
runs happen, in which order, and whether a failing or empty run fails the
script. That catches what reading the script can't: an early exit, the steps
moved into a function nothing calls, `set +e`, a reordering. Each such change
is applied to a copy of the script below and must be caught, and so is each
way of breaking counted_cargo_test in scripts/ci/lib.sh.
Needs Python 3.11 or later (tomllib); the test that reads Cargo.toml is skipped
on an older one.
Run by scripts/ci.sh: python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import importlib.util

try:
    import tomllib
except ImportError:  # Python < 3.11
    tomllib = None

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
FEATURE = "raikiri"
DOM_TESTS = os.path.join(ROOT, "crates", "dom", "tests")

# count-tests.py's own rule for which files the feature run must run.
_spec = importlib.util.spec_from_file_location("count_tests", os.path.join(HERE, "count-tests.py"))
count_tests = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(count_tests)

CLIPPY = "clippy -p surimono-dom --all-targets --features raikiri --locked -- -D warnings"
TEST = "test -p surimono-dom --features raikiri --locked --no-fail-fast"
DEFAULT_CLIPPY = "clippy --all-targets --locked -- -D warnings"
DEFAULT_TEST = "test --locked --no-fail-fast"
RUBY_CLIPPY = "clippy --workspace --all-targets --locked -- -D warnings"
RUBY_TEST = "test --workspace --locked --no-fail-fast"
SOAK = "test --release --workspace --exclude surimono-ruby --locked -- --ignored soak --nocapture"
LAYOUT_SOAK = ("test --release -p surimono-dom --features raikiri --locked --test layout_soak "
               "-- --ignored soak --nocapture")

# The cargo stub. Logs `cargo <args>` to $STUB_LOG, then applies the first
# rule of $STUB_RULES ([[exact args, exit code, stdout], ...]) whose args
# match; no rule: no output, exit 0.
CARGO_STUB = r'''#!{python}
import json, os, sys
args = " ".join(sys.argv[1:])
with open(os.environ["STUB_LOG"], "a") as f:
    f.write("cargo " + args + "\n")
for want, code, out in json.loads(os.environ.get("STUB_RULES", "[]")):
    if want == args:
        sys.stdout.write(out)
        sys.exit(code)
'''
# python3 -m unittest (ci.sh's own tests: this file) is logged, not run;
# anything else (count-tests.py) runs in the real interpreter.
PYTHON_STUB = r'''#!{python}
import os, sys
if "-m" in sys.argv and "unittest" in sys.argv:
    with open(os.environ["STUB_LOG"], "a") as f:
        f.write("python3 -m unittest\n")
    sys.exit(0)
os.execv({python!r}, [{python!r}] + sys.argv[1:])
'''
# Logs "<name>" or "<name> <args>", with no trailing space for a call without
# arguments.
LOGGER = '#!/bin/sh\nprintf "%s\\n" "{name}${1+ $*}" >> "$STUB_LOG"\n'


def gated_files():
    """Stems of crates/dom/tests/*.rs the feature run must run: count-tests.py's
    rule (feature_files), so the test and the check can't drift apart."""
    return count_tests.feature_files(FEATURE, DOM_TESTS)


COUNTED_STEP = ("counted_cargo_test --gated-by raikiri crates/dom/tests -- \\\n"
                "  -p surimono-dom --features raikiri --locked --no-fail-fast\n")


def libtest(sections):
    """Cargo's test output for [(path, passed, ignored), ...], with the color
    codes CI's CARGO_TERM_COLOR=always adds to cargo's own lines."""
    out = []
    for path, passed, ignored in sections:
        out.append(f"\x1b[1m\x1b[32m     Running\x1b[0m {path} (target/debug/deps/x-0123)\n")
        out.append(f"\nrunning {passed} tests\n")
        out.append(f"test result: ok. {passed} passed; 0 failed; {ignored} ignored; "
                   f"0 measured; 0 filtered out; finished in 0.01s\n\n")
    return "".join(out)


def feature_run_output(passed=1, skip=None, gated=True, empty=None):
    """A feature test run: default-feature targets, then every gated one
    (none without `gated`, all but `skip`), each with `passed` tests and one
    ignored; `empty` names one that ran no test at all."""
    sections = [("unittests src/lib.rs", 59, 2), ("tests/documents.rs", 91, 0)]
    for name in gated_files() if gated else []:
        if name == empty:
            sections.append((f"tests/{name}.rs", 0, 0))
        elif name != skip:
            sections.append((f"tests/{name}.rs", passed, 1))
    return libtest(sections)


SOAK_OUTPUT = libtest([("tests/page_soak.rs", 1, 0), ("tests/page.rs", 0, 0),
                       ("tests/soak.rs", 1, 0), ("tests/soak.rs", 4, 0)])
LAYOUT_SOAK_OUTPUT = libtest([("tests/layout_soak.rs", 1, 0)])


class Tree:
    """A scratch copy of the scripts with stubs first on PATH."""

    def __init__(self, ci_sh=None, soak_sh=None, lib_sh=None):
        self.dir = tempfile.mkdtemp(prefix="p2-pkg-10-ci-")
        d = self.dir
        os.makedirs(os.path.join(d, "scripts", "ci"))
        os.makedirs(os.path.join(d, "bindings", "ruby"))
        shutil.copytree(DOM_TESTS, os.path.join(d, "crates", "dom", "tests"))
        shutil.copy(os.path.join(ROOT, "scripts/ci/count-tests.py"), os.path.join(d, "scripts/ci/count-tests.py"))
        self.write("scripts/ci/lib.sh", lib_sh if lib_sh is not None else read("scripts/ci/lib.sh"))
        self.write("scripts/ci.sh", ci_sh if ci_sh is not None else read("scripts/ci.sh"))
        self.write("scripts/ci/soak.sh", soak_sh if soak_sh is not None else read("scripts/ci/soak.sh"))
        for sub in ("fixtures.sh", "wpt-runner.sh", "page-check.sh"):
            self.write(f"scripts/ci/{sub}", LOGGER.replace("{name}", "script " + sub))
        self.bin = os.path.join(d, "bin")
        os.makedirs(self.bin)
        py = sys.executable
        self.write("bin/cargo", CARGO_STUB.replace("{python}", py))
        self.write("bin/python3", PYTHON_STUB.replace("{python!r}", repr(py)).replace("{python}", py))
        self.write("bin/bundle", LOGGER.replace("{name}", "bundle"))
        self.write("bin/rustup", "#!/bin/sh\nexit 0\n")
        self.write("bin/cargo-deny", "#!/bin/sh\nexit 0\n")
        self.write("bin/ruby", f'#!/bin/sh\nprintf %s "{self.bin}"\n')
        self.log = os.path.join(d, "calls.log")

    def write(self, rel, text):
        path = os.path.join(self.dir, rel)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        os.chmod(path, 0o755)

    def run(self, script, args=(), rules=()):
        """(exit code, calls, output) of one run of `script` in the tree."""
        open(self.log, "w").close()
        env = {k: v for k, v in os.environ.items()
               if k not in ("GITHUB_ACTIONS", "CI_JOB_OUT", "CARGO_TARGET_DIR", "RUSTUP_TOOLCHAIN")}
        env.update(PATH=self.bin + os.pathsep + env.get("PATH", "/usr/bin:/bin"),
                   STUB_LOG=self.log, STUB_RULES=json.dumps(list(rules)),
                   LIBCLANG_PATH="/stub", TMPDIR=self.dir)
        p = subprocess.run(["bash", os.path.join(self.dir, script), *args], env=env,
                           capture_output=True, text=True, timeout=120)
        with open(self.log, encoding="utf-8") as f:
            calls = [l.rstrip() for l in f]
        return p.returncode, calls, p.stdout + p.stderr

    def run_bash(self, script, rules=()):
        """(exit code, calls, output) of `bash -c script` with the stubs."""
        path = os.path.join(self.dir, "case.sh")
        with open(path, "w", encoding="utf-8") as f:
            f.write(script + "\n")
        return self.run("case.sh", (), rules)

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)


def read(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read()


def ok_rules():
    return [[TEST, 0, feature_run_output()], [SOAK, 0, SOAK_OUTPUT],
            [LAYOUT_SOAK, 0, LAYOUT_SOAK_OUTPUT]]


class _Found(Exception):
    pass


class Problems(list):
    """A list of problems that, with `first`, stops the scenarios at the
    first one (a mutant needs one to be caught; this keeps the test quick)."""

    def __init__(self, first):
        super().__init__()
        self.first = first

    def append(self, item):
        super().append(item)
        if self.first:
            raise _Found


def ci_problems(ci_sh=None, first=False, lib_sh=None):
    """Every way scripts/ci.sh (or the given text of it) breaks the feature
    steps' contract, over each path through it; empty when it holds."""
    problems = Problems(first)
    tree = Tree(ci_sh=ci_sh, lib_sh=lib_sh)
    try:
        for args, default in ((["--no-ruby"], [DEFAULT_CLIPPY, DEFAULT_TEST]),
                              ([], [RUBY_CLIPPY, RUBY_TEST]),
                              (["--no-deny"], [RUBY_CLIPPY, RUBY_TEST])):
            code, calls, out = tree.run("scripts/ci.sh", args, ok_rules())
            label = " ".join(args) or "(no options)"
            cargo = [c[len("cargo "):] for c in calls if c.startswith("cargo ")]
            if code != 0:
                problems.append(f"{label}: exit {code} with every step green")
            want = default + [CLIPPY, TEST]
            for cmd in want:
                if cargo.count(cmd) != 1:
                    problems.append(f"{label}: `cargo {cmd}` ran {cargo.count(cmd)} times")
            if all(cargo.count(c) == 1 for c in want):
                order = [cargo.index(c) for c in want]
                if order != sorted(order):
                    problems.append(f"{label}: feature steps out of order: {order}")
            if any("--all-features" in c for c in cargo):
                problems.append(f"{label}: a run turns on --all-features")
        # A failing feature step fails the script, and nothing after it runs.
        # The failing test run prints real-looking output, so it is cargo's
        # status, not the count, that must fail it.
        for cmd, out in ((CLIPPY, ""), (TEST, feature_run_output())):
            rules = [[cmd, 101, out]] + ok_rules()
            code, calls, _ = tree.run("scripts/ci.sh", ["--no-ruby"], rules)
            if code == 0:
                problems.append(f"a failing `cargo {cmd}` leaves the script green")
            if "script wpt-runner.sh" in calls:
                problems.append(f"steps after a failing `cargo {cmd}` still ran")
        # A feature run that runs no feature-only test, or skips a gated file, fails.
        for name, output in (("0 feature-only tests", feature_run_output(passed=0)),
                             ("no gated test file", feature_run_output(gated=False)),
                             (f"tests/{gated_files()[0]}.rs missing",
                              feature_run_output(skip=gated_files()[0])),
                             ("raikiri_build at 0 tests beside the others",
                              feature_run_output(empty="raikiri_build"))):
            rules = [[TEST, 0, output]] + ok_rules()
            code, _, _ = tree.run("scripts/ci.sh", ["--no-ruby"], rules)
            if code == 0:
                problems.append(f"a feature test run with {name} leaves the script green")
    except _Found:
        pass
    finally:
        tree.close()
    return problems


def soak_problems(soak_sh=None, first=False, lib_sh=None):
    """Every way scripts/ci/soak.sh (or the given text) breaks its contract."""
    problems = Problems(first)
    tree = Tree(soak_sh=soak_sh, lib_sh=lib_sh)
    try:
        code, calls, _ = tree.run("scripts/ci/soak.sh", (), ok_rules())
        cargo = [c[len("cargo "):] for c in calls if c.startswith("cargo ")]
        if code != 0:
            problems.append(f"exit {code} with both soaks green")
        if cargo != [SOAK, LAYOUT_SOAK]:
            problems.append(f"cargo runs {cargo}, expected the workspace soak, then the layout soak")
        bad = {
            "a failing layout soak": [[LAYOUT_SOAK, 101, LAYOUT_SOAK_OUTPUT]],
            "a layout soak filtered to 0 tests": [[LAYOUT_SOAK, 0, libtest([("tests/layout_soak.rs", 0, 0)])]],
            "a layout soak run without its target": [[LAYOUT_SOAK, 0, ""]],
            "a failing workspace soak": [[SOAK, 101, SOAK_OUTPUT]],
            "a workspace soak missing a soak": [[SOAK, 0, libtest([("tests/page_soak.rs", 1, 0), ("tests/soak.rs", 4, 0)])]],
        }
        for name, rules in bad.items():
            code, _, _ = tree.run("scripts/ci/soak.sh", (), rules + ok_rules())
            if code == 0:
                problems.append(f"{name} leaves soak.sh green")
    except _Found:
        pass
    finally:
        tree.close()
    return problems


class CountTestsTest(unittest.TestCase):
    """scripts/ci/count-tests.py on its own."""

    def count(self, text, *args):
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as f:
            f.write(text)
        try:
            return subprocess.run([sys.executable, "-B", "-I", os.path.join(HERE, "count-tests.py"),
                                   f.name, *args], capture_output=True, text=True).returncode
        finally:
            os.unlink(f.name)

    def test_a_target_counts_its_runs_in_every_crate(self):
        self.assertEqual(self.count(SOAK_OUTPUT, "--target", "soak", "--min-passed", "5"), 0)
        self.assertEqual(self.count(SOAK_OUTPUT, "--target", "soak", "--min-passed", "6"), 1)
        self.assertEqual(self.count(SOAK_OUTPUT, "--min-passed", "6"), 0)

    def test_an_absent_target_fails_even_with_min_zero(self):
        self.assertEqual(self.count(SOAK_OUTPUT, "--target", "layout_soak", "--min-passed", "0"), 1)

    def test_an_empty_run_fails(self):
        self.assertEqual(self.count(""), 1)
        self.assertEqual(self.count(libtest([("tests/x.rs", 0, 3)])), 1)

    def test_the_gated_files_are_found_and_needed(self):
        self.assertGreaterEqual(len(gated_files()), 2)
        self.assertIn("raikiri_build", gated_files())
        self.assertIn("layout_soak", gated_files())
        args = ("--gated-by", FEATURE, DOM_TESTS)
        self.assertEqual(self.count(feature_run_output(), *args), 0)
        self.assertEqual(self.count(feature_run_output(passed=0), *args), 1)
        self.assertEqual(self.count(feature_run_output(skip="raikiri_build"), *args), 1)
        self.assertEqual(self.count(feature_run_output(), "--gated-by", "no-such-feature", DOM_TESTS), 1)

    def test_each_gated_file_needs_a_test_of_its_own(self):
        # #189: raikiri_build at 0 tests beside the other files' tests is red;
        # a file with only ignored tests (layout_cost) is not.
        args = ("--gated-by", FEATURE, DOM_TESTS)
        self.assertEqual(self.count(feature_run_output(empty="raikiri_build"), *args), 1)
        only_ignored = feature_run_output(skip="layout_cost") + libtest([("tests/layout_cost.rs", 0, 2)])
        self.assertEqual(self.count(only_ignored, *args), 0)

    def test_every_form_of_the_feature_gate_is_found(self):
        forms = {
            "whole": '#![cfg(feature = "raikiri")]\n#[test]\nfn a() {}\n',
            "all": '#![cfg(all(feature = "raikiri", target_os = "linux"))]\n',
            "item": '#[test]\nfn a() {}\n#[cfg(feature = "raikiri")]\n#[test]\nfn b() {}\n',
            "attr": '#[test]\n#[cfg_attr(not(feature="raikiri"), ignore)]\nfn a() {}\n',
            "any": '    #[cfg(any(test, feature  =  "raikiri"))]\n    let x = 1;\n',
            "comment": '// gated on feature = "raikiri" one day\n//! run with --features raikiri\n#[test]\nfn a() {}\n',
            "other": '#![cfg(feature = "raikiri-extra")]\n',
            "plain": '#[test]\nfn a() {}\n',
        }
        d = tempfile.mkdtemp(prefix="p2-pkg-10-forms-")
        try:
            for name, text in forms.items():
                with open(os.path.join(d, name + ".rs"), "w", encoding="utf-8") as f:
                    f.write(text)
            self.assertEqual(count_tests.feature_files(FEATURE, d), ["all", "any", "attr", "item", "whole"])
        finally:
            shutil.rmtree(d)

    def test_the_repos_gated_files(self):
        # Five files gated as a whole, and layout_real.rs, which gates items.
        self.assertEqual(gated_files(), ["layout_chrome", "layout_cost", "layout_real",
                                         "layout_soak", "layout_time", "raikiri_build"])


class CiFeatureStepsTest(unittest.TestCase):
    def test_ci_sh_runs_the_feature_steps_on_every_path(self):
        self.assertEqual(ci_problems(), [])

    def test_the_stub_run_catches_a_broken_ci_sh(self):
        script = read("scripts/ci.sh")
        clippy_step = 'step "clippy (surimono-dom, raikiri feature)"\n'
        test_step = 'step "test (surimono-dom, raikiri feature)"\n'
        self.assertIn(clippy_step, script)
        self.assertIn(test_step, script)
        start = script.index(clippy_step)
        end = script.index("scripts/ci/wpt-runner.sh")
        steps = script[start:end]
        test_part = steps[steps.index(test_step):]
        mutants = {
            "an early exit": script.replace(clippy_step, "exit 0\n" + clippy_step),
            "the steps in an uncalled function": script.replace(steps, "feature() {\n" + steps + "}\n"),
            "set +e": script.replace("set -euo pipefail", "set -uo pipefail\nset +e"),
            "the test before the clippy run": script.replace(steps, test_part + steps[:steps.index(test_step)]),
            "the feature steps before the default runs": script.replace(steps, "").replace(
                'scripts/ci/fixtures.sh\n', 'scripts/ci/fixtures.sh\n' + steps),
            "the test step deleted": script.replace(test_part, ""),
            "a plain cargo test (no count)": script.replace(
                test_part, test_step + "cargo " + TEST + "\n"),
            "--no-ruby skipping the steps": script.replace(steps, 'if [ "$run_ruby" = 1 ]; then\n' + steps + "fi\n"),
            "the steps behind an unset variable": script.replace(
                steps, 'if [ -n "${SURIMONO_FEATURE_CI:-}" ]; then\n' + steps + "fi\n"),
            "a failing clippy remembered, later steps run": script.replace(
                "--features raikiri --locked -- -D warnings\n", "--features raikiri --locked -- -D warnings || rc=1\n")
                .replace('step "ok"\n', 'step "ok"\nexit ${rc:-0}\n'),
            "|| true after the counted step": script.replace(COUNTED_STEP, COUNTED_STEP.rstrip("\n") + " || true\n"),
            "the counted step without the feature": script.replace(
                "-p surimono-dom --features raikiri --locked --no-fail-fast", "-p surimono-dom --locked --no-fail-fast"),
            "the test in a subshell, uncounted": script.replace(
                COUNTED_STEP, "(cargo " + TEST + " | cat)\n"),
        }
        for name, text in mutants.items():
            with self.subTest(name):
                self.assertNotEqual(text, script, "the mutation changed nothing")
                self.assertNotEqual(ci_problems(text, first=True), [], f"{name} was not caught")

    def test_ci_sh_holds_without_pipefail(self):
        # counted_cargo_test takes cargo's status itself, so a failing or
        # empty feature test run still fails ci.sh when pipefail is off.
        script = read("scripts/ci.sh").replace("set -euo pipefail", "set -euo pipefail\nset +o pipefail")
        self.assertIn("set +o pipefail", script)
        self.assertEqual(ci_problems(script), [])

    def test_no_run_turns_on_every_feature_of_every_crate(self):
        # --all-features would also turn on the other crates' test-only
        # features (surimono-core's test-hooks) in what the tests link.
        code = "\n".join(l for l in read("scripts/ci.sh").splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("--all-features", code)

    @unittest.skipIf(tomllib is None, "needs Python 3.11 (tomllib)")
    def test_the_feature_the_steps_name_exists_and_is_off_by_default(self):
        with open(os.path.join(ROOT, "crates", "dom", "Cargo.toml"), "rb") as f:
            features = tomllib.load(f).get("features", {})
        self.assertIn(FEATURE, features)
        self.assertNotIn(FEATURE, features.get("default", []))


class SoakStepsTest(unittest.TestCase):
    def test_soak_sh_runs_both_soaks_and_counts_them(self):
        self.assertEqual(soak_problems(), [])

    def test_soak_sh_holds_without_pipefail(self):
        script = read("scripts/ci/soak.sh").replace("set -euo pipefail", "set -euo pipefail\nset +o pipefail")
        self.assertIn("set +o pipefail", script)
        self.assertEqual(soak_problems(script), [])

    def test_the_stub_run_catches_a_broken_soak_sh(self):
        script = read("scripts/ci/soak.sh")
        layout = script[script.index('step "soak (surimono-dom'):]
        mutants = {
            "the layout soak deleted": script.replace(layout, ""),
            "an early exit": script.replace(layout, "exit 0\n" + layout),
            "the layout soak without the feature": script.replace(
                "--release -p surimono-dom --features raikiri", "--release -p surimono-dom"),
            "the layout soak uncounted": script.replace(
                "counted_cargo_test --target layout_soak --min-passed 1 -- \\\n  ", "cargo test "),
            "set +e": script.replace("set -euo pipefail", "set -uo pipefail\nset +e"),
            "the workspace minimum lowered to 1": script.replace("--min-passed 6", "--min-passed 1"),
            "|| true after the layout soak": script.replace(
                "-- --ignored soak --nocapture\n", "-- --ignored soak --nocapture || true\n")
                .replace("--ignored soak --nocapture || true\n# surimono", "--ignored soak --nocapture\n# surimono"),
        }
        for name, text in mutants.items():
            with self.subTest(name):
                self.assertNotEqual(text, script, "the mutation changed nothing")
                self.assertNotEqual(soak_problems(text, first=True), [], f"{name} was not caught")


def lib_problems(lib_sh=None):
    """Every way counted_cargo_test (in lib.sh, or the given text of it) gets
    its status or its clean-up wrong, called directly whatever the caller's
    shell options are; empty when it holds."""
    problems = []
    tree = Tree(lib_sh=lib_sh)
    good = libtest([("tests/a.rs", 3, 0)])
    rules = {"ok": [["test ok", 0, good]], "fail": [["test fail", 101, good]],
             "empty": [["test empty", 0, libtest([("tests/a.rs", 0, 0)])]]}
    # (name, shell options, call, which cargo, the status `echo rc=$?` must print)
    cases = [
        ("failing cargo, no errexit, no pipefail", "set +e +o pipefail", "plain", "fail", 101),
        ("failing cargo inside if", "set -euo pipefail", "if", "fail", 101),
        ("failing cargo before &&", "set -euo pipefail", "and", "fail", 101),
        ("0 tests, no errexit", "set +e +o pipefail", "plain", "empty", 1),
        ("0 tests inside if", "set -euo pipefail", "if", "empty", 1),
        ("good run inside if", "set -euo pipefail", "if", "ok", 0),
        ("good run, no errexit, no pipefail", "set +e +o pipefail", "plain", "ok", 0),
    ]
    calls = {
        "plain": "counted_cargo_test --target a -- {c}; echo rc=$?",
        "if": "if counted_cargo_test --target a -- {c}; then echo rc=0; else echo rc=$?; fi",
        "and": "counted_cargo_test --target a -- {c} && echo rc=0 || echo rc=$?",
    }
    try:
        for name, opts, call, which, want in cases + [
                ("failing cargo under errexit", "set -euo pipefail", "plain", "fail", None)]:
            script = f'cd "{tree.dir}"; {opts}; . scripts/ci/lib.sh; ' + calls[call].format(c=which)
            code, _, out = tree.run_bash(script, rules[which])
            got = re.findall(r"^rc=(\d+)$", out, re.M)
            if want is None:
                # errexit: the caller stops at the failing call, with cargo's status.
                if code != 101 or got:
                    problems.append(f"{name}: exit {code}, printed {got}; expected exit 101 there")
            elif got != [str(want)]:
                problems.append(f"{name}: status {got}, expected {want}")
            left = [f for f in os.listdir(tree.dir) if f.startswith("cargo-test.")]
            if left:
                problems.append(f"{name}: temporary files left: {left}")
                for f in left:
                    os.unlink(os.path.join(tree.dir, f))
    finally:
        tree.close()
    return problems


class CountedCargoTestTest(unittest.TestCase):
    def test_the_status_is_right_whatever_the_callers_options(self):
        self.assertEqual(lib_problems(), [])

    def test_a_broken_counted_cargo_test_is_caught(self):
        lib = read("scripts/ci/lib.sh")
        mutants = {
            "the check's status ignored": lib.replace("|| check_rc=$?", "|| true"),
            "cargo's status swallowed": lib.replace('cargo test "$@" 2>&1 || cargo_rc=$?', 'cargo test "$@" 2>&1 || true'),
            "the check skipped": lib.replace('if [ "$cargo_rc" = 0 ]; then', "if false; then"),
            "no clean-up": lib.replace('rm -f "$log" "$log.rc"', ":"),
            "cargo's status from the pipeline (needs pipefail)": lib.replace(
                '{ cargo_rc=0; cargo test "$@" 2>&1 || cargo_rc=$?; echo "$cargo_rc" > "$log.rc"; } | tee "$log" || true\n'
                '  cargo_rc="$(cat "$log.rc" 2>/dev/null || echo 1)"',
                'cargo test "$@" 2>&1 | tee "$log" && cargo_rc=0 || cargo_rc=$?'),
        }
        for name, text in mutants.items():
            with self.subTest(name):
                self.assertNotEqual(text, lib, "the mutation changed nothing")
                self.assertNotEqual(lib_problems(text), [], f"{name} was not caught")


if __name__ == "__main__":
    unittest.main()
