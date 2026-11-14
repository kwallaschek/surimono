"""Tests that scripts/ci.sh always checks surimono-dom's `raikiri` feature
(P2-PKG-09). The feature is off by default, so without these two steps CI
builds none of the code behind it: clippy never reads it and
crates/dom/tests/raikiri_build.rs compiles to 0 tests, which is how #189
merged. The steps must run on every path through the script (with and without
--no-ruby, whatever else is skipped), for that one package only. Run by
scripts/ci.sh: python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'"""

import os
import re
import tomllib
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CLIPPY = "cargo clippy -p surimono-dom --all-targets --features raikiri --locked -- -D warnings"
TEST = "cargo test -p surimono-dom --features raikiri --locked --no-fail-fast"

OPEN = re.compile(r"^(if|for|while|until|case)\b")
CLOSE = re.compile(r"^(fi|done|esac)\b")


def unconditional_commands(script):
    """The command lines of a shell script that no if, loop or case encloses:
    lines at depth 0, with the command after a `step "..."; ` prefix, if any."""
    depth, out = 0, []
    for raw in script.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if CLOSE.match(line):
            depth -= 1
            continue
        if depth == 0 and not OPEN.match(line):
            out.append(re.sub(r'^step "[^"]*";\s*', "", line))
        # A one-line `if ...; then ...; fi` opens and closes on the same line.
        if OPEN.match(line) and not re.search(r";\s*(fi|done|esac)\s*$", line):
            depth += 1
    return out


class CiFeatureStepsTest(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(ROOT, "scripts", "ci.sh"), encoding="utf-8") as f:
            self.script = f.read()

    def test_the_reader_finds_only_what_nothing_encloses(self):
        sample = 'a\nif x; then\n  b\n  for i in 1; do\n    c\n  done\nelse\n  d\nfi\n' \
                 'if y; then e; fi\nstep "f"; g --h\n# i\n'
        self.assertEqual(unconditional_commands(sample), ["a", "g --h"])

    def test_the_feature_is_linted_and_tested_on_every_path(self):
        commands = unconditional_commands(self.script)
        self.assertIn(CLIPPY, commands)
        self.assertIn(TEST, commands)
        # After the default-feature runs, so their results come first.
        self.assertLess(commands.index(CLIPPY), commands.index(TEST))

    def test_no_run_turns_on_every_feature_of_every_crate(self):
        # --all-features would also turn on the other crates' test-only
        # features (surimono-core's test-hooks) in what the tests link.
        code = "\n".join(l for l in self.script.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("--all-features", code)

    def test_the_feature_the_steps_name_exists_and_is_off_by_default(self):
        with open(os.path.join(ROOT, "crates", "dom", "Cargo.toml"), "rb") as f:
            features = tomllib.load(f).get("features", {})
        self.assertIn("raikiri", features)
        self.assertNotIn("raikiri", features.get("default", []))


if __name__ == "__main__":
    unittest.main()
