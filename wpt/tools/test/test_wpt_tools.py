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


def sub(name: str, status: str) -> dict:
    return {"name": name, "status": status}


def entry(name: str, status: str, subtests: list[dict]) -> dict:
    return {"slice": "s", "test": name, "file": name, "status": status, "subtests": subtests}


@unittest.skipUnless(shutil.which("node"), "needs node")
class RunsTests(unittest.TestCase):
    """runs.mjs: what chrome_baseline.mjs --runs and --merge write (wpt/README.md, "Flaky subtests")."""

    TOOLS = pathlib.Path(__file__).resolve().parents[1]

    def intersect(self, runs: list[list[dict]]) -> list[dict]:
        script = "import { intersectRuns } from './runs.mjs';\nlet s='';process.stdin.on('data',d=>s+=d).on('end',()=>console.log(JSON.stringify(intersectRuns(JSON.parse(s)))));"
        r = subprocess.run(["node", "--input-type=module", "-e", script], cwd=self.TOOLS, input=json.dumps(runs), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr), (0, ""))
        return json.loads(r.stdout)

    def test_one_run_is_written_as_it_was(self):
        run = [entry("a.html", "OK", [sub("x", "PASS"), sub("y", "FAIL")])]
        self.assertEqual(self.intersect([run]), run)

    def test_the_same_result_every_time_has_no_runs_field(self):
        run = [entry("a.html", "OK", [sub("x", "PASS"), sub("y", "FAIL")])]
        self.assertEqual(self.intersect([run] * 5), run)

    def test_a_subtest_counts_only_if_it_passes_in_every_run(self):
        runs = [[entry("a.html", "OK", [sub("x", "PASS"), sub("y", s)])] for s in ("PASS", "PASS", "FAIL", "PASS", "TIMEOUT")]
        [t] = self.intersect(runs)
        self.assertEqual(t["status"], "OK")
        self.assertEqual(t["subtests"], [sub("x", "PASS"), {"name": "y", "status": "FAIL", "runs": ["PASS", "PASS", "FAIL", "PASS", "TIMEOUT"]}])

    def test_a_subtest_missing_from_a_run_is_notrun_there(self):
        runs = [[entry("a.html", "OK", [sub("x", "PASS")])], [entry("a.html", "OK", [sub("x", "PASS"), sub("z", "PASS")])]]
        [t] = self.intersect(runs)
        self.assertEqual(t["subtests"], [sub("x", "PASS"), {"name": "z", "status": "NOTRUN", "runs": ["NOTRUN", "PASS"]}])

    def test_a_test_that_does_not_finish_every_time_is_left_out_whole(self):
        runs = [[entry("a.html", s, [sub("x", "PASS")])] for s in ("OK", "TIMEOUT", "OK")]
        [t] = self.intersect(runs)
        self.assertEqual((t["status"], t["runs"]), ("TIMEOUT", ["OK", "TIMEOUT", "OK"]))
        self.assertEqual(t["subtests"], [sub("x", "PASS")])

    def test_a_test_missing_from_a_run_is_not_ok(self):
        runs = [[entry("a.html", "OK", []), entry("b.html", "OK", [])], [entry("a.html", "OK", [])]]
        merged = self.intersect(runs)
        self.assertEqual([(t["test"], t["status"]) for t in merged], [("a.html", "OK"), ("b.html", "NOTRUN")])

    def subtest_of(self, reports: list[dict]) -> list[dict]:
        script = "import { subtestOf } from './runs.mjs';\nlet s='';process.stdin.on('data',d=>s+=d).on('end',()=>console.log(JSON.stringify(JSON.parse(s).map(subtestOf))));"
        r = subprocess.run(["node", "--input-type=module", "-e", script], cwd=self.TOOLS, input=json.dumps(reports), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr), (0, ""))
        return json.loads(r.stdout)

    def test_a_failure_keeps_its_message_and_a_pass_has_none(self):
        long = "\U0001F600" * 400  # code points, not UTF-16 units, are counted
        got = self.subtest_of([
            {"name": "p", "status": 0, "message": None},
            {"name": "p2", "status": 0, "message": "stale"},
            {"name": "f", "status": 1, "message": "assert_equals: expected 1 but got 2"},
            {"name": "t", "status": 2, "message": None},
            {"name": "n", "status": 3, "message": ""},
            {"name": "c", "status": 4, "message": "needs x"},
            {"name": "u", "status": 9, "message": long},
        ])
        self.assertEqual(got[:6], [
            sub("p", "PASS"), sub("p2", "PASS"),
            {"name": "f", "status": "FAIL", "message": "assert_equals: expected 1 but got 2"},
            sub("t", "TIMEOUT"), sub("n", "NOTRUN"),
            {"name": "c", "status": "PRECONDITION_FAILED", "message": "needs x"},
        ])
        self.assertEqual((got[6]["status"], got[6]["message"]), ("FAIL", "\U0001F600" * 300))

    def test_the_message_written_is_the_one_of_the_status_written(self):
        def run(status, message=None):
            s = sub("y", status) | ({"message": message} if message else {})
            return [entry("a.html", "OK", [s])]

        [t] = self.intersect([run("PASS"), run("FAIL", "second"), run("TIMEOUT", "third")])
        self.assertEqual(t["subtests"], [{"name": "y", "status": "FAIL", "message": "second", "runs": ["PASS", "FAIL", "TIMEOUT"]}])
        [t] = self.intersect([run("FAIL", "one"), run("FAIL", "two")])
        self.assertEqual(t["subtests"], [{"name": "y", "status": "FAIL", "message": "one"}])
        [t] = self.intersect([run("PASS"), run("PASS")])
        self.assertEqual(t["subtests"], [sub("y", "PASS")])

    def test_compat_scores_a_flaky_subtest_as_left_out(self):
        sys.path.insert(0, str(self.TOOLS.parents[1] / "fixtures" / "tools"))
        import compat  # noqa: PLC0415

        runs = [[entry("a.html", "OK", [sub("x", "PASS"), sub("y", s)])] for s in ("PASS", "FAIL")]
        [t] = self.intersect(runs)
        ours = {"status": "OK", "subtests": [sub("x", "PASS"), sub("y", "PASS")]}
        self.assertEqual(compat.wpt_passed(t, ours), (1, 1))
        [left_out] = self.intersect([[entry("b.html", s, [sub("x", "PASS")])] for s in ("OK", "TIMEOUT")])
        self.assertEqual(compat.wpt_passed(left_out, ours)[1], 0)


class BaselineRuleTests(unittest.TestCase):
    """The committed baseline is the intersection of five runs."""

    WPT = pathlib.Path(__file__).resolve().parents[2]

    def test_the_committed_baseline_says_how_many_runs(self):
        baseline = json.loads((self.WPT / "baseline" / "chrome.json").read_text(encoding="utf-8"))
        self.assertGreaterEqual(baseline.get("runs", 1), 5)

    @unittest.skipUnless(shutil.which("node"), "needs node")
    def test_the_tool_refuses_to_write_the_baseline_from_fewer_runs(self):
        tool = self.WPT / "tools" / "chrome_baseline.mjs"
        for extra in (["--runs", "1"], ["--merge", "a.json", "b.json"]):
            r = subprocess.run(["node", str(tool), "--from", "/nonexistent", *extra], capture_output=True, text=True, cwd=self.WPT)
            self.assertEqual(r.returncode, 1, r.stderr)
            self.assertIn("every one of 5 runs", r.stderr)


@unittest.skipUnless(shutil.which("node"), "needs node")
class MergeGuardTests(unittest.TestCase):
    """chrome_baseline.mjs --merge and --update refuse what would miscount runs.
    None of these starts a browser: --merge reads files only."""

    WPT = pathlib.Path(__file__).resolve().parents[2]
    TOOL = WPT / "tools" / "chrome_baseline.mjs"
    URL = "dom/nodes/Node-cloneNode-XMLDocument.html"
    COMMIT = "8b4b06ff53a47f1041e0e59f894d9e929963904c"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = pathlib.Path(self.tmp.name)
        self.made = 0

    def run_file(self, **header) -> pathlib.Path:
        self.made += 1
        data = {"schema": 1, "wpt_commit": self.COMMIT, "chromium": "153.0.8010.12", "runs": 1, "mode": "default",
                "run_ids": [f"2026-11-16T00:00:0{self.made}.000Z#0000000{self.made}"]}
        data |= header
        data = {k: v for k, v in data.items() if v is not None}
        data.setdefault("tests", [entry(self.URL, "OK", [sub("x", "PASS")]) | {"slice": "dom-nodes", "file": self.URL}])
        path = self.dir / f"run{self.made}.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def tool(self, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run(["node", str(self.TOOL), "--filter", "Node-cloneNode-XMLDocument", *extra], capture_output=True, text=True, cwd=self.WPT)

    def merge(self, files, out=None, *extra: str) -> subprocess.CompletedProcess:
        return self.tool("--merge", *map(str, files), "--out", str(out or self.dir / "out.json"), *extra)

    def refused(self, r: subprocess.CompletedProcess, text: str):
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn(text, r.stderr)

    def test_two_distinct_runs_merge_and_say_which(self):
        a, b = self.run_file(), self.run_file()
        r = self.merge([a, b])
        self.assertEqual(r.returncode, 0, r.stderr)
        out = json.loads((self.dir / "out.json").read_text())
        self.assertEqual((out["runs"], out["mode"]), (2, "default"))
        self.assertEqual(out["run_ids"], [json.loads(a.read_text())["run_ids"][0], json.loads(b.read_text())["run_ids"][0]])

    def test_the_same_file_twice_is_refused(self):
        a = self.run_file()
        self.refused(self.merge([a, self.run_file(), a]), "again; each run counts once")
        link = self.dir / "link.json"
        link.symlink_to(a)
        self.refused(self.merge([a, link]), "again; each run counts once")

    def test_a_copy_of_a_run_is_refused(self):
        a = self.run_file()
        copy = self.dir / "copy.json"
        shutil.copy(a, copy)
        self.refused(self.merge([a, copy]), "are the same run")

    def test_two_runs_with_the_same_results_are_two_runs(self):
        # Two of the five runs of 16 Nov are byte-identical but for their ids.
        self.assertEqual(self.merge([self.run_file(), self.run_file()]).returncode, 0)

    def test_a_run_without_an_id_is_refused(self):
        self.refused(self.merge([self.run_file(), self.run_file(run_ids=None)]), "has no run id")

    def test_runs_of_two_modes_are_refused(self):
        self.refused(self.merge([self.run_file(), self.run_file(mode="no-pointer")]), "runs of two modes never combine")
        self.refused(self.merge([self.run_file(), self.run_file(mode=None)]), "runs of two modes never combine")

    def test_another_chromium_or_commit_is_refused(self):
        self.refused(self.merge([self.run_file(), self.run_file(chromium="154.0.0.0")]), "is from Chromium 154.0.0.0")
        self.refused(self.merge([self.run_file(), self.run_file(wpt_commit="0" * 40)]), "is from Chromium")

    def test_a_merged_file_is_refused_as_input(self):
        self.refused(self.merge([self.run_file(), self.run_file(runs=2)]), "already 2 runs combined, not a single run")
        # One with ids and one from before ids: named as combined, not as lacking an id.
        self.refused(self.merge([self.run_file(), self.run_file(runs=2, run_ids=["a", "b"])]), "already 2 runs combined, not a single run")
        self.refused(self.merge([self.run_file(), self.run_file(runs=5, run_ids=None)]), "already 5 runs combined, not a single run")
        self.refused(self.merge([self.run_file(), self.run_file(runs=None, run_ids=["a", "b"])]), "already 2 runs combined, not a single run")

    def test_the_committed_baseline_is_refused_as_input(self):
        r = self.tool("--merge", str(self.run_file()), str(self.WPT / "baseline" / "chrome.json"), "--out", str(self.dir / "out.json"))
        self.refused(r, "already 5 runs combined, not a single run")

    def test_gzipped_runs_merge_like_plain_ones(self):
        import gzip  # noqa: PLC0415

        a, b = self.run_file(), self.run_file()
        packed = self.dir / "b.json.gz"
        packed.write_bytes(gzip.compress(b.read_bytes(), mtime=0))
        plain, gz = self.dir / "plain.json", self.dir / "gz.json"
        self.assertEqual(self.merge([a, b], plain).returncode, 0)
        self.assertEqual(self.merge([a, packed], gz).returncode, 0)
        self.assertEqual(plain.read_bytes(), gz.read_bytes())

    def test_a_damaged_input_is_named_not_a_stack_trace(self):
        import gzip  # noqa: PLC0415

        a, b = self.run_file(), self.run_file()
        packed = gzip.compress(b.read_bytes(), mtime=0)
        truncated = self.dir / "truncated.json.gz"
        truncated.write_bytes(packed[: len(packed) // 2])
        plain_gz = self.dir / "plain.json.gz"
        plain_gz.write_bytes(b.read_bytes())
        unnamed = self.dir / "unnamed.json"
        unnamed.write_bytes(packed)
        not_json = self.dir / "garbage.json"
        not_json.write_text("{", encoding="utf-8")
        cases = [
            (truncated, "is damaged gzip data"),
            (plain_gz, "is named .gz but isn't gzip data"),
            (unnamed, "is gzip data but isn't named .gz"),
            (not_json, "isn't a JSON run file"),
            (self.dir / "missing.json", "can't read"),
        ]
        for path, text in cases:
            with self.subTest(path.name):
                r = self.merge([a, path])
                self.refused(r, f"{path} {text}" if text != "can't read" else f"can't read {path}")
                self.assertNotIn("    at ", r.stderr)  # no stack frames

    def test_a_run_missing_a_test_is_refused(self):
        self.refused(self.merge([self.run_file(), self.run_file(tests=[])]), "doesn't hold the tests")

    def test_duplicate_subtest_names_are_refused(self):
        dup = [entry(self.URL, "OK", [sub("x", "PASS"), sub("x", "FAIL")]) | {"slice": "dom-nodes", "file": self.URL}]
        self.refused(self.merge([self.run_file(), self.run_file(tests=dup)]), "twice")

    def test_update_never_mixes_rules_or_modes(self):
        prior = self.run_file(runs=5, run_ids=["p1", "p2", "p3", "p4", "p5"])
        self.refused(self.merge([self.run_file(), self.run_file()], prior, "--update"), "never mixes two rules")
        prior = self.run_file(runs=2, mode="no-pointer", run_ids=["p1", "p2"])
        self.refused(self.merge([self.run_file(), self.run_file()], prior, "--update"), "never mixes two modes")
        prior = self.run_file(runs=2, run_ids=["p1", "p2"])
        r = self.merge([self.run_file(), self.run_file()], prior, "--update")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(prior.read_text())["earlier_run_ids"], ["p1", "p2"])

    def test_the_baseline_through_a_symlink_is_still_the_baseline(self):
        baseline = self.WPT / "baseline" / "chrome.json"
        before = baseline.read_bytes()
        link = self.dir / "chrome.json"
        link.symlink_to(baseline)
        try:
            self.refused(self.merge([self.run_file(), self.run_file()], link), "every one of 5 runs")
            five = [self.run_file(mode="no-pointer") for _ in range(5)]
            self.refused(self.merge(five, link), "made in the default mode")
        finally:
            if baseline.read_bytes() != before:  # a broken guard must not leave the real file changed
                baseline.write_bytes(before)
                self.fail("the guard let a merge write wpt/baseline/chrome.json")


@unittest.skipUnless(shutil.which("node"), "needs node")
class CommittedRunsTests(unittest.TestCase):
    """wpt/baseline/runs/ holds the single runs wpt/baseline/chrome.json is merged
    from (wpt/README.md, "Flaky subtests"), so the file can be rebuilt from the
    repository: no browser, no network."""

    WPT = pathlib.Path(__file__).resolve().parents[2]

    def test_merging_the_committed_runs_gives_the_baseline_byte_for_byte(self):
        runs = sorted((self.WPT / "baseline" / "runs").glob("run*.json.gz"))
        self.assertEqual([p.name for p in runs], [f"run{i}.json.gz" for i in range(1, 6)])
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp) / "chrome.json"
            r = subprocess.run(["node", str(self.WPT / "tools" / "chrome_baseline.mjs"), "--merge", *map(str, runs), "--out", str(out)],
                               capture_output=True, text=True, cwd=self.WPT)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(out.read_bytes(), (self.WPT / "baseline" / "chrome.json").read_bytes())

    def test_the_driver_log_has_the_start_times_the_ids_cite(self):
        import gzip  # noqa: PLC0415

        log = (self.WPT / "baseline" / "runs" / "driver.log").read_text(encoding="utf-8")
        for i in range(1, 6):
            data = json.loads(gzip.decompress((self.WPT / "baseline" / "runs" / f"run{i}.json.gz").read_bytes()))
            [run_id] = data["run_ids"]
            started = run_id.split("T")[1][:8]  # UTC
            hh, mm, ss = map(int, started.split(":"))
            jst = f"{(hh + 9) % 24:02d}:{mm:02d}:{ss:02d}"
            self.assertIn(f"pass {i} start {jst} ", log)
            self.assertEqual(run_id.split("#")[1], f"legacy-run{i}")


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
