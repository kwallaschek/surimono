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
    def run_check(self, packages, about=ABOUT, deny=DENY, shipped=None):
        """`shipped`: the names cargo tree lists; default every package."""
        if shipped is None:
            shipped = [p["name"] for p in packages] or ["root"]
        with tempfile.TemporaryDirectory() as tmp:
            about_path = os.path.join(tmp, "about.toml")
            deny_path = os.path.join(tmp, "deny.toml")
            shipped_path = os.path.join(tmp, "shipped.txt")
            with open(about_path, "w") as f:
                f.write(about)
            with open(deny_path, "w") as f:
                f.write(deny)
            with open(shipped_path, "w") as f:
                # As `cargo tree --prefix none -f '{p}'` prints them.
                f.write("".join(f"{n} v1.0.0 (*)\n" for n in shipped))
            stdin = io.StringIO(json.dumps({"packages": packages}))
            err = io.StringIO()
            argv = ["check-mpl.py", about_path, deny_path, shipped_path]
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
        selectors = package("selectors", "MPL-2.0")
        code, err = self.run_check([selectors, package("newmpl", "MPL-2.0")])
        self.assertEqual(code, 1)
        self.assertIn("newmpl", err)
        code, err = self.run_check([selectors, package("dual", "MIT OR MPL-2.0")])
        self.assertEqual(code, 0, err)

    def test_dev_only_mpl_crate_needs_deny_but_not_about(self):
        deny = (
            '[licenses]\nexceptions = [{ name = "selectors", allow = ["MPL-2.0"] },'
            ' { name = "stylo", allow = ["MPL-2.0"] }]\n'
        )
        packages = [package("selectors", "MPL-2.0"), package("stylo", "MPL-2.0")]
        code, err = self.run_check(packages, deny=deny, shipped=["selectors"])
        self.assertEqual(code, 0, err)
        self.assertIn("shipped: selectors", err)
        # Not in deny.toml either: fails even though it isn't shipped.
        code, err = self.run_check(packages, shipped=["selectors"])
        self.assertEqual(code, 1)
        self.assertIn("stylo 1.0.0 is MPL-2.0 but not one of the crates", err)
        # Shipped but not in about.toml: cargo-about would fail.
        code, err = self.run_check(packages, deny=deny)
        self.assertEqual(code, 1)
        self.assertIn("missing from", err)

    def test_about_names_only_shipped_crates_allowed_by_deny(self):
        # Listed in about.toml but not shipped: cargo-about would only warn.
        code, err = self.run_check([package("x", "MIT")])
        self.assertEqual(code, 1)
        self.assertIn("doesn't link: ['selectors']", err)
        code, err = self.run_check(
            [package("selectors", "MPL-2.0")], deny="[licenses]\nexceptions = []\n"
        )
        self.assertEqual(code, 1)
        self.assertIn("doesn't allow: ['selectors']", err)
        code, err = self.run_check(
            [package("selectors", "MPL-2.0")],
            about='accepted = ["MIT", "MPL-2.0"]\n\n[selectors]\naccepted = ["MPL-2.0"]\n',
        )
        self.assertEqual(code, 1)
        self.assertIn("every crate", err)

    def test_empty_shipped_list_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = [os.path.join(tmp, n) for n in ("about.toml", "deny.toml", "shipped.txt")]
            for path, text in zip(paths, (ABOUT, DENY, "")):
                with open(path, "w") as f:
                    f.write(text)
            with mock.patch.object(sys, "argv", ["check-mpl.py", *paths]):
                with self.assertRaises(SystemExit) as e:
                    check_mpl.main()
            self.assertIn("lists no crates", str(e.exception.code))

if __name__ == "__main__":
    unittest.main()
