"""Tests for verify.py's offline embedded check (no network).

    python3 -I -m unittest discover -s fixtures/real-world -p 'test_*.py'
"""

import contextlib
import hashlib
import io
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import verify  # noqa: E402

BUNDLE = b"globalThis.entry = function () { return '<svg/>'; };\n"

MANIFEST = f"""
schema = 1
phase = 1

[[library]]
name = "demo"
version = "1.0.0"
status = "required"
license = "MIT"
license_files = ["LICENSE"]
npm_tarball = "https://registry.npmjs.org/demo/-/demo-1.0.0.tgz"
integrity = "sha512-AAAA"
tarball_sha256 = "00"
dist_file = "dist/demo.js"
dist_sha256 = "00"
dist_bytes = 1

[library.embedded]
file = "crates/islands/bundles/demo/demo.min.js"
sha256 = "{hashlib.sha256(BUNDLE).hexdigest()}"
bytes = {len(BUNDLE)}
built_by = "test"
built_from = ["demo 1.0.0"]
license_files = ["LICENSE"]
recorded_from = "test"
"""


class EmbeddedOnly(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.tmp.name)
        self.manifest = self.dir / "manifest.toml"
        self.manifest.write_text(MANIFEST)
        self.root = self.dir / "checkout"
        self.bundle = self.root / "crates/islands/bundles/demo/demo.min.js"
        # --embedded-only must never download.
        self._fetch = verify.fetch
        verify.fetch = lambda *a, **k: self.fail("--embedded-only downloaded a tarball")

    def tearDown(self) -> None:
        verify.fetch = self._fetch
        self.tmp.cleanup()

    def run_verify(self) -> tuple[int, str]:
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = verify.main(["--embedded-only", "--manifest", str(self.manifest), "--root", str(self.root)])
        return code, out.getvalue()

    def write_bundle(self, data: bytes, licence: bool = True) -> None:
        self.bundle.parent.mkdir(parents=True, exist_ok=True)
        self.bundle.write_bytes(data)
        if licence:
            (self.bundle.parent / "LICENSE").write_text("MIT\n")

    def test_matching_bundle_passes(self) -> None:
        self.write_bundle(BUNDLE)
        code, out = self.run_verify()
        self.assertEqual(code, 0, out)
        self.assertIn("checked", out)

    def test_rebuilt_bundle_fails_with_its_hash_and_size(self) -> None:
        changed = BUNDLE + b"// rebuilt\n"
        self.write_bundle(changed)
        code, out = self.run_verify()
        self.assertEqual(code, 1, out)
        self.assertIn(hashlib.sha256(changed).hexdigest(), out)
        self.assertIn(f"{len(changed)} bytes", out)

    def test_missing_licence_fails(self) -> None:
        self.write_bundle(BUNDLE, licence=False)
        code, out = self.run_verify()
        self.assertEqual(code, 1, out)
        self.assertIn("licence file LICENSE missing", out)

    def test_missing_bundle_fails(self) -> None:
        code, out = self.run_verify()
        self.assertEqual(code, 1, out)
        self.assertIn("is missing", out)

    def test_unknown_library_name_fails(self) -> None:
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = verify.main(["--embedded-only", "--manifest", str(self.manifest), "nope"])
        self.assertEqual(code, 1)


class RealManifest(unittest.TestCase):
    def test_every_embedded_entry_matches_this_checkout(self) -> None:
        """The drift guard: what the manifest records is what crates/islands embeds."""
        import tomllib

        manifest = tomllib.loads((pathlib.Path(verify.HERE) / "manifest.toml").read_text(encoding="utf-8"))
        for lib in manifest["library"]:
            if "embedded" in lib:
                errors, note = verify.check_embedded(lib)
                self.assertEqual(errors, [], lib["name"])
                self.assertIn("checked", note)


class VendoredTests(unittest.TestCase):
    def lib(self, data: bytes, **over) -> dict:
        return {
            "license_files": ["LICENSE"],
            "vendored": [
                {"file": "lib/x.js", "member": "dist/x.js", "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)},
                {"file": "lib/LICENSE", "member": "LICENSE", "sha256": hashlib.sha256(b"L").hexdigest(), "bytes": 1},
            ],
            **over,
        }

    def tree(self, data: bytes) -> tempfile.TemporaryDirectory:
        tmp = tempfile.TemporaryDirectory()
        root = pathlib.Path(tmp.name)
        (root / "lib").mkdir()
        (root / "lib/x.js").write_bytes(data)
        (root / "lib/LICENSE").write_bytes(b"L")
        return tmp

    def test_matching_files_pass(self):
        with self.tree(b"js") as tmp:
            self.assertEqual(verify.check_vendored(self.lib(b"js"), pathlib.Path(tmp)), [])

    def test_changed_file_fails(self):
        with self.tree(b"edited") as tmp:
            errors = verify.check_vendored(self.lib(b"js"), pathlib.Path(tmp))
        self.assertTrue(any("differs from the manifest" in e for e in errors), errors)

    def test_missing_file_fails(self):
        with self.tree(b"js") as tmp:
            (pathlib.Path(tmp) / "lib/x.js").unlink()
            errors = verify.check_vendored(self.lib(b"js"), pathlib.Path(tmp))
        self.assertTrue(any("is missing" in e for e in errors), errors)

    def test_licence_must_be_vendored_or_explained(self):
        with self.tree(b"js") as tmp:
            root = pathlib.Path(tmp)
            self.assertTrue(verify.check_vendored(self.lib(b"js", license_files=["NOTICE"]), root))
            self.assertEqual(verify.check_vendored(self.lib(b"js", license_files=[]), root), ["no licence file and no license_pending"])
            self.assertEqual(verify.check_vendored(self.lib(b"js", license_files=[], license_pending={"task": "T"}), root), [])

    def test_licence_vendored_from_the_source_repo_counts(self):
        with self.tree(b"js") as tmp:
            lib = self.lib(b"js", license_files=[])
            lib["vendored"][0]["license"] = True
            self.assertEqual(verify.check_vendored(lib, pathlib.Path(tmp)), [])

    def test_fonts_licence_must_be_vendored(self):
        with self.tree(b"js") as tmp:
            errors = verify.check_vendored(self.lib(b"js", fonts_license_files=["fonts/OFL.txt"]), pathlib.Path(tmp))
        self.assertEqual(errors, ["fonts licence file fonts/OFL.txt isn't vendored"])


MIT = "MIT License\n\nCopyright (c) Someone\n\nPermission is hereby granted, free of charge\n"


class PendingLicenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "NOTE.md").write_text("why", encoding="utf-8")
        self.lib = {"license_pending": {"file": "LICENSE.md", "source_url": "https://example.test/LICENSE.md", "note": "NOTE.md", "task": "P1-CF-32"}}

    def test_missing_file_is_named_with_its_task(self):
        self.assertEqual(verify.check_pending(self.lib, self.root, False), ([], "licence file pending (P1-CF-32)"))

    def test_missing_file_fails_a_release(self):
        errors, _ = verify.check_pending(self.lib, self.root, True)
        self.assertIn("P1-CF-32", errors[0])

    def test_arrived_file_is_accepted(self):
        (self.root / "LICENSE.md").write_text(MIT, encoding="utf-8")
        self.assertEqual(verify.check_pending(self.lib, self.root, False), ([], "licence file arrived: pin it and close P1-CF-32"))

    def test_arrived_file_must_still_be_pinned_for_a_release(self):
        (self.root / "LICENSE.md").write_text(MIT, encoding="utf-8")
        errors, _ = verify.check_pending(self.lib, self.root, True)
        self.assertIn("not pinned", errors[0])

    def test_arrived_file_must_be_a_licence(self):
        (self.root / "LICENSE.md").write_text("404: Not Found", encoding="utf-8")
        errors, _ = verify.check_pending(self.lib, self.root, False)
        self.assertEqual(errors, ["LICENSE.md isn't an MIT licence text"])

    def test_the_note_must_stay(self):
        (self.root / "NOTE.md").unlink()
        errors, _ = verify.check_pending(self.lib, self.root, False)
        self.assertEqual(errors, ["licence note NOTE.md is missing"])

    def test_no_pending_entry_is_fine(self):
        self.assertEqual(verify.check_pending({}, self.root, True), ([], ""))


if __name__ == "__main__":
    unittest.main()
