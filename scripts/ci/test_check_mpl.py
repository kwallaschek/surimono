"""Tests for check-mpl.py (run by scripts/ci.sh):
python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'"""

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check_mpl", os.path.join(HERE, "check-mpl.py"))
check_mpl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_mpl)

CRATES_IO = check_mpl.CRATES_IO
ABOUT = 'accepted = ["MIT"]\n\n[selectors]\naccepted = ["MPL-2.0"]\n'
DENY = '[licenses]\nexceptions = [{ name = "selectors", allow = ["MPL-2.0"] }]\n'


def package(name, licence, source=CRATES_IO):
    return {"name": name, "version": "1.0.0", "license": licence, "source": source}


class MplOnly(unittest.TestCase):
    def test_expressions(self):
        for expr, want in [
            ("MPL-2.0", True),
            ("MPL-2.0 AND MIT", True),
            ("(MIT OR Apache-2.0) AND MPL-2.0", True),
            ("MIT OR MPL-2.0", False),
            ("Apache-2.0/MPL-2.0", False),
            ("MPL-2.0 OR (MIT AND Apache-2.0)", False),
            ("MIT", False),
            ("", False),
        ]:
            self.assertEqual(check_mpl.mpl_only(expr), want, expr)


class Main(unittest.TestCase):
    def run_check(self, packages, about=ABOUT, deny=DENY):
        with tempfile.TemporaryDirectory() as tmp:
            about_path = os.path.join(tmp, "about.toml")
            deny_path = os.path.join(tmp, "deny.toml")
            with open(about_path, "w") as f:
                f.write(about)
            with open(deny_path, "w") as f:
                f.write(deny)
            stdin = io.StringIO(json.dumps({"packages": packages}))
            err = io.StringIO()
            argv = ["check-mpl.py", about_path, deny_path]
            with mock.patch.object(sys, "argv", argv), mock.patch.object(sys, "stdin", stdin):
                with redirect_stderr(err):
                    try:
                        check_mpl.main()
                        code = 0
                    except SystemExit as e:
                        code = e.code
            return code, err.getvalue()

    def test_allowed_crate_from_crates_io_passes(self):
        code, err = self.run_check([package("selectors", "MPL-2.0"), package("x", "MIT")])
        self.assertEqual(code, 0, err)
        self.assertIn("selectors 1.0.0", err)

    def test_patched_mpl_crate_fails(self):
        for source in [None, "git+https://github.com/servo/stylo?rev=abc#abc"]:
            code, err = self.run_check([package("selectors", "MPL-2.0", source)])
            self.assertEqual(code, 1)
            self.assertIn("not unmodified from crates.io", err)

    def test_unlisted_mpl_crate_fails_unless_it_has_another_choice(self):
        code, err = self.run_check([package("newmpl", "MPL-2.0")])
        self.assertEqual(code, 1)
        self.assertIn("newmpl", err)
        code, err = self.run_check([package("dual", "MIT OR MPL-2.0")])
        self.assertEqual(code, 0, err)

    def test_lists_must_match_and_mpl_must_not_be_global(self):
        code, err = self.run_check([], deny="[licenses]\nexceptions = []\n")
        self.assertEqual(code, 1)
        self.assertIn("differ", err)
        code, err = self.run_check([], about='accepted = ["MIT", "MPL-2.0"]\n\n[selectors]\naccepted = ["MPL-2.0"]\n')
        self.assertEqual(code, 1)
        self.assertIn("every crate", err)


if __name__ == "__main__":
    unittest.main()
