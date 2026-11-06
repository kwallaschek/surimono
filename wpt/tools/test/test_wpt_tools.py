"""Tests for wpt/tools on a made-up checkout (no network, no real WPT).

    python3 -I -m unittest discover -s wpt/tools/test -p 'test_*.py'
"""

import contextlib
import io
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import api_needs  # noqa: E402
import vendor  # noqa: E402

TESTHARNESS = '<script src="/resources/testharness.js"></script><script src="/resources/testharnessreport.js"></script>'
FILES = {
    "LICENSE.md": "licence",
    "resources/testharness.js": "// harness",
    "dom/nodes/Node-a.html": f'{TESTHARNESS}<script src="helper.js"></script><script>test(() => {{}}, "a");</script>',
    "dom/nodes/helper.js": "// helper",
    "dom/nodes/Node-b.any.js": "// META: script=/common/x.js\ntest(() => {});",
    "common/x.js": "// x",
    "dom/nodes/reftest.html": "<p>no harness</p>",
    "dom/nodes/x-ref.html": f"{TESTHARNESS}",
    "dom/nodes/big.html": TESTHARNESS + "x" * 500,
}


def checkout(root: pathlib.Path) -> str:
    for rel, text in FILES.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    run = lambda *a: subprocess.run(["git", "-C", str(root), *a], check=True, capture_output=True, text=True)  # noqa: E731
    run("init", "-q")
    run("add", "-A")
    run("-c", "user.name=t", "-c", "user.email=t@example.test", "commit", "-qm", "x")
    return run("rev-parse", "HEAD").stdout.strip()


def slices(commit: str) -> dict:
    return {"commit": commit, "source": "https://example.test/wpt", "max_file_bytes": 400, "slice": [{"id": "dom-nodes", "paths": ["dom/nodes"]}]}


class VendorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = pathlib.Path(self.tmp.name, "wpt")
        self.src.mkdir()
        self.out = pathlib.Path(self.tmp.name, "out")
        self.out.mkdir()
        self.commit = checkout(self.src)

    def test_copies_tests_and_what_they_name(self):
        vendor.vendor(self.src, self.out, slices(self.commit))
        m = tomllib.loads((self.out / "manifest.toml").read_text(encoding="utf-8"))
        self.assertEqual(sorted(t["file"] for t in m["test"]), ["dom/nodes/Node-a.html", "dom/nodes/Node-b.any.js"])
        self.assertEqual(sorted(f["path"] for f in m["file"]), sorted(["LICENSE.md", "common/x.js", "dom/nodes/Node-a.html", "dom/nodes/Node-b.any.js", "dom/nodes/helper.js", "resources/testharness.js"]))
        self.assertEqual([s["file"] for s in m["skipped"]], ["dom/nodes/big.html"])
        self.assertEqual(m["commit"], self.commit)

    def manifest(self):
        return tomllib.loads((self.out / "manifest.toml").read_text(encoding="utf-8"))

    def add(self, rel, text):
        p = self.src / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")

    def test_symlinked_dependency_is_not_copied(self):
        secret = pathlib.Path(self.tmp.name, "secret.txt")
        secret.write_text("private", encoding="utf-8")
        (self.src / "dom/nodes/evil.js").symlink_to(secret)
        self.add("dom/nodes/Node-c.html", f'{TESTHARNESS}<script src="evil.js"></script>')
        vendor.vendor(self.src, self.out, slices(self.commit))
        self.assertFalse((self.out / "tests/dom/nodes/evil.js").exists())
        self.assertIn("dom/nodes/evil.js", [m["file"] for m in self.manifest()["missing"]])

    def test_dependency_through_a_symlinked_directory_is_not_copied(self):
        outside = pathlib.Path(self.tmp.name, "outside")
        outside.mkdir()
        (outside / "x.js").write_text("private", encoding="utf-8")
        (self.src / "dom/link").symlink_to(outside)
        self.add("dom/nodes/Node-c.html", f'{TESTHARNESS}<script src="/dom/link/x.js"></script>')
        vendor.vendor(self.src, self.out, slices(self.commit))
        self.assertFalse((self.out / "tests/dom/link/x.js").exists())

    def test_support_files_and_folders_come_with_the_tests(self):
        (self.src / "dom/nodes/frames").mkdir()
        (self.src / "dom/nodes/frames/a.xml").write_text("<a/>")
        (self.src / "dom/nodes/frames/b.svg").write_text("<svg/>")
        (self.src / "common").mkdir(exist_ok=True)
        (self.src / "common/blank.html").write_text("")
        (self.src / "common/secret.html").symlink_to("/etc/hostname")
        commit = checkout(self.src)
        sl = slices(commit) | {"support": ["common/blank.html", "common/secret.html", "common/absent.html", "dom/nodes/frames"]}
        vendor.vendor(self.src, self.out, sl)
        for rel in ("common/blank.html", "dom/nodes/frames/a.xml", "dom/nodes/frames/b.svg"):
            self.assertTrue((self.out / "tests" / rel).is_file(), rel)
        self.assertFalse((self.out / "tests/common/secret.html").exists())
        manifest = tomllib.loads((self.out / "manifest.toml").read_text())
        self.assertEqual({m["file"] for m in manifest["missing"]}, {"common/absent.html", "common/secret.html"})
        self.assertNotIn("dom/nodes/frames/a.xml", [t["file"] for t in manifest["test"]])

    def test_report_hook_is_never_vendored_however_it_is_named(self):
        self.assertEqual(vendor.dependencies("dom/nodes/a.html", '<script src="../../resources/testharnessreport.js"></script>'), [])

    def test_xht_and_xml_are_tests(self):
        self.add("dom/nodes/a.xht", TESTHARNESS)
        self.add("dom/nodes/b.xml", TESTHARNESS)
        self.add("dom/nodes/c-iframe.xml", "<x/>")
        vendor.vendor(self.src, self.out, slices(self.commit))
        files = [t["file"] for t in self.manifest()["test"]]
        self.assertIn("dom/nodes/a.xht", files)
        self.assertIn("dom/nodes/b.xml", files)
        self.assertNotIn("dom/nodes/c-iframe.xml", files)

    def test_ids_of_generated_tests_and_variants(self):
        self.assertEqual(vendor.urls("a/b.any.js", ""), ["a/b.any.html"])
        self.assertEqual(vendor.urls("a/b.window.js", ""), ["a/b.window.html"])
        self.assertEqual(vendor.urls("a/b.any.js", "// META: global=window,worker\n"), ["a/b.any.html"])
        self.assertEqual(vendor.urls("a/b.any.js", "// META: global=dedicatedworker\n"), [])
        self.assertEqual(vendor.urls("a/b.any.js", "// META: variant=?x\n// META: variant=?y\n"), ["a/b.any.html?x", "a/b.any.html?y"])
        self.assertEqual(vendor.urls("a/b.html", '<meta name="variant" content="?1-10"><meta name=variant content="?11-20">'), ["a/b.html?1-10", "a/b.html?11-20"])

    def test_worker_only_test_is_left_out(self):
        self.add("dom/nodes/w.any.js", "// META: global=serviceworker\ntest(() => {});")
        vendor.vendor(self.src, self.out, slices(self.commit))
        self.assertNotIn("dom/nodes/w.any.js", [t["file"] for t in self.manifest()["test"]])

    def test_missing_harness_is_fatal_and_keeps_what_was_there(self):
        vendor.vendor(self.src, self.out, slices(self.commit))
        (self.src / "resources/testharness.js").unlink()
        with self.assertRaises(SystemExit):
            vendor.vendor(self.src, self.out, slices(self.commit))
        self.assertTrue((self.out / "tests/dom/nodes/Node-a.html").exists())

    def test_oversized_harness_is_fatal(self):
        self.add("resources/testharness.js", "x" * 500)
        with self.assertRaises(SystemExit):
            vendor.vendor(self.src, self.out, slices(self.commit))

    def test_a_bad_dependency_leaves_the_tests_alone(self):
        vendor.vendor(self.src, self.out, slices(self.commit))
        self.add("dom/nodes/Node-c.html", f'{TESTHARNESS}<script src="/../x.js"></script>')
        self.add("dom/nodes/Node-d.html", f'{TESTHARNESS}<script src="../../../x.js"></script>')
        with self.assertRaises(ValueError):
            vendor.vendor(self.src, self.out, slices(self.commit))
        self.assertTrue((self.out / "tests/dom/nodes/Node-a.html").exists())

    def test_a_checkout_at_another_commit_is_refused(self):
        with self.assertRaises(SystemExit):
            vendor.vendor(self.src, self.out, slices("0" * 40))

    def test_check_finds_edits_and_strays(self):
        vendor.vendor(self.src, self.out, slices(self.commit))
        (self.out / "slices.toml").write_text(f'commit = "{self.commit}"\n', encoding="utf-8")
        self.assertEqual(vendor.check(self.out), [])
        (self.out / "tests/dom/nodes/helper.js").write_text("edited", encoding="utf-8")
        (self.out / "tests/extra.js").write_text("x", encoding="utf-8")
        errors = vendor.check(self.out)
        self.assertIn("dom/nodes/helper.js differs from the manifest", errors)
        self.assertIn("extra.js isn't in the manifest", errors)

    def test_check_passes_before_vendoring(self):
        self.assertEqual(vendor.check(pathlib.Path(self.tmp.name, "empty")), [])

    def test_check_refuses_tests_without_a_manifest(self):
        (self.out / "tests").mkdir()
        self.assertEqual(vendor.check(self.out), ["wpt/tests exists but manifest.toml doesn't"])

    def test_check_notices_a_bumped_pin(self):
        vendor.vendor(self.src, self.out, slices(self.commit))
        (self.out / "slices.toml").write_text('commit = "new"\n', encoding="utf-8")
        self.assertTrue(any("slices.toml pins new" in e for e in vendor.check(self.out)))

    def test_a_dependency_cannot_leave_the_wpt_root(self):
        with self.assertRaises(ValueError):
            vendor.dependencies("a.html", '<script src="../../etc/x.js"></script>')


IDL = """
interface Node { readonly attribute unsigned short nodeType; Node appendChild(Node node); readonly attribute DOMString length; };
interface mixin ParentNode { Element? querySelector(DOMString selectors); };
interface Element : Node { undefined setAttribute(DOMString name, DOMString value); };
interface CharacterData : Node { readonly attribute unsigned long length; };
interface Event { constructor(DOMString type); };
"""


class SubstitutionTests(unittest.TestCase):
    TOOLS = pathlib.Path(__file__).resolve().parents[1]

    @unittest.skipUnless(shutil.which("node"), "needs node")
    def test_the_baseline_gives_what_the_vectors_say(self):
        script = """
import { readFileSync } from 'node:fs';
import { substitute } from './substitute.mjs';
for (const v of JSON.parse(readFileSync('sub-vectors.json', 'utf8'))) {
  let got = null;
  try { got = substitute(v.text, new URL(v.url, 'http://web-platform.test:8000/')); } catch {}
  if (got !== v.want) console.log(JSON.stringify([v, got]));
}
"""
        r = subprocess.run(["node", "--input-type=module", "-e", script], cwd=self.TOOLS, capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stdout, r.stderr), (0, "", ""))

    def test_the_vectors_refuse_some_and_keep_html_escaping(self):
        vectors = json.loads((self.TOOLS / "sub-vectors.json").read_text(encoding="utf-8"))
        self.assertTrue(any(v["want"] is None for v in vectors))
        self.assertTrue(any("&amp;" in (v["want"] or "") for v in vectors))


class ApiNeedsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "dom.idl").write_text(IDL, encoding="utf-8")
        self.names, self.members = api_needs.load_idl(self.root)

    def run_on(self, files: dict[str, str]) -> dict:
        for rel, text in files.items():
            (self.root / rel).write_text(text, encoding="utf-8")
        tests = [{"slice": "s", "file": f, "needs": []} for f in files]
        return api_needs.analyse(tests, self.root, self.names, self.members)

    def test_reads_interfaces_and_members(self):
        self.assertEqual(self.names, {"Node", "ParentNode", "Element", "CharacterData", "Event"})
        self.assertEqual(self.members["appendChild"], {"Node"})
        self.assertEqual(self.members["length"], {"Node", "CharacterData"})

    def test_shared_member_names_are_no_evidence(self):
        self.assertEqual(api_needs.needs_of("x.length", self.names, self.members), set())
        self.assertEqual(api_needs.needs_of("x.querySelector('a'); new Event('e')", self.names, self.members), {"ParentNode", "Event"})

    def test_greedy_order_prefers_what_completes_most(self):
        r = self.run_on({
            "a.html": "<script>test(()=>{});test(()=>{});test(()=>{});x.appendChild(y)</script>",
            "b.html": "<script>test(()=>{});new Event('e');x.appendChild(y)</script>",
        })
        self.assertEqual([o["interface"] for o in r["order"]], ["Node", "Event"])
        self.assertEqual(r["order"][0]["completes"], 3)
        self.assertEqual(r["order"][-1]["reachable_after"], 4)

    def test_workers_are_set_aside(self):
        r = self.run_on({"a.html": '<script>new Worker("x");test(()=>{})</script>'})
        self.assertEqual(r["blocked"], {"worker": 1})
        self.assertEqual(r["subtests"], 0)
        self.assertEqual(r["tests"][0]["blocked_by"], ["worker"])

    def test_frames_and_xml_pages_count(self):
        r = self.run_on({"a.html": '<iframe src="x"></iframe><script>test(()=>{})</script>', "c.xht": "<script>test(()=>{})</script>"})
        self.assertEqual(r["blocked"], {})
        self.assertEqual(r["subtests"], 2)

    def test_substitution_and_idl_tests_are_set_aside(self):
        r = self.run_on({"a.sub.html": "<script>test(()=>{})</script>", "b.html": "<script>idl_test([],[],()=>{})</script>"})
        self.assertEqual(r["blocked"], {"idl files from the server": 1, "server-side substitution": 1})

    def test_without_a_manifest_it_says_so(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = api_needs.main(["--manifest", str(self.root / "none.toml")])
        self.assertEqual(code, 1)
        self.assertIn("vendor.py", err.getvalue())


if __name__ == "__main__":
    unittest.main()
