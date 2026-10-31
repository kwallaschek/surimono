"""Tests for fixtures/tools/compat.py on a made-up repository tree.

    python3 -I -m unittest discover -s fixtures/tools/test -p 'test_*.py'
"""

import importlib.util
import itertools
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("compat", HERE.parent / "compat.py")
compat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compat)

# Verdicts compat.py and lib/report.mjs must agree on (report.test.mjs reads them too).
VERDICTS = json.loads((HERE / "verdict-cases.json").read_text(encoding="utf-8"))

HOSTILE = """
[limits]
memory_bytes = 1
time_ms = 1
stack_bytes = 1
max_wall_ms = 4000

[[case]]
id = "loop"
file = "loop.js"
expect = "TimeLimit"
about = "x"

[[case]]
id = "ok"
file = "ok.js"
expect_any = ["Ok", "Exception"]
output = "fine"
about = "x"

[[case]]
id = "req"
file = "req.js"
expect = "Exception"
error = "ReferenceError"
about = "x"

[[case]]
id = "two"
steps = [{ file = "a.js", expect = "Ok", output = "a" }, { file = "b.js", expect = "Ok", output = "b" }]
about = "x"
"""

REAL_WORLD = """
schema = 1
[[library]]
name = "echarts"
version = "6.1.0"
status = "required"
license = "Apache-2.0"
[[library]]
name = "vega"
version = "6.4.0"
status = "required"
license = "BSD-3-Clause"
"""


def score(subject, passed, failed=(), text_pass=None, worst=0.0, colour=None, hue=None, text_missing=()):
    s = {"subject": subject, "pass": passed, "worst_window": worst, "pages": [{"pass": passed, "failed": list(failed)}]}
    if colour is not None:
        s["worst_colour_window"] = colour
    if hue is not None:
        s["worst_hue_window"] = hue
    if text_pass is not None:
        s["text"] = {"pass": text_pass, "missing": list(text_missing), "extra": []}
    return s


class CompatTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        for name, renderer in [("echarts-a", "echarts"), ("echarts-b", "echarts"), ("vega-a", "vega")]:
            d = root / "fixtures" / "islands" / name
            d.mkdir(parents=True)
            (d / "meta.toml").write_text(f'renderer = "{renderer}"\nislands = [{{ renderer = "{renderer}", width = 8, height = 8 }}]\n')
        (root / "fixtures" / "hostile").mkdir(parents=True)
        (root / "fixtures" / "hostile" / "manifest.toml").write_text(HOSTILE)
        (root / "fixtures" / "real-world").mkdir(parents=True)
        (root / "fixtures" / "real-world" / "manifest.toml").write_text(REAL_WORLD)
        self.root = root
        self.old_root = compat.ROOT
        compat.ROOT = root

    def tearDown(self):
        compat.ROOT = self.old_root
        self.tmp.cleanup()

    def write_json(self, name, data):
        p = self.root / name
        p.write_text(json.dumps(data))
        return p

    def test_hand_written_sections_survive(self):
        existing = f"# old\n\n{compat.BEGIN}\n\n## Known gaps\n\nmy notes, kept\n\n{compat.END}\n"
        out = compat.generate([], None, None, "c", existing)
        self.assertIn("my notes, kept", out)
        self.assertEqual(out.count(compat.BEGIN), 1)
        # Regenerating from its own output changes nothing.
        self.assertEqual(compat.generate([], None, None, "c", out), out)

    def test_first_run_keeps_everything_from_known_gaps_on(self):
        existing = "# Compatibility\n\nold summary\n\n## Known gaps\n\ngap one\n\n## Printing gaps\n\ngap two\n"
        out = compat.generate([], None, None, "c", existing)
        self.assertIn("gap one", out)
        self.assertIn("gap two", out)
        self.assertNotIn("old summary", out)

    def test_hostile_verdicts_come_from_the_manifest(self):
        results = self.write_json(
            "hostile.json",
            {
                "commit": "abc",
                "cases": [
                    {"id": "loop", "step": None, "outcome": "TimeLimit", "wall_ms": 2001, "pass": False},
                    {"id": "ok", "step": None, "outcome": "Ok", "output": "wrong", "wall_ms": 1, "pass": True},
                    {"id": "req", "step": None, "outcome": "Exception", "message": "TypeError: x", "wall_ms": 1},
                    {"id": "two", "step": 1, "outcome": "Ok", "output": "a", "wall_ms": 5000},
                ],
            },
        )
        text = "\n".join(compat.hostile_section(results))
        self.assertIn("1 of 5 cases end as the manifest expects", text)
        self.assertIn("| loop | TimeLimit | TimeLimit (2001 ms) | pass |", text)
        self.assertIn("output 'wrong', expected 'fine'", text)  # the file's own "pass" is ignored
        self.assertIn("expected ReferenceError", text)
        self.assertIn("over max_wall_ms 4000", text)
        self.assertIn("| two step 2 | Ok | not run | not run |", text)

    def test_fixture_and_real_world_status(self):
        scores = [
            score("echarts-a island 0 (printed by chrome)", True),
            score("echarts-a island 0 (printed by fulgur)", True),
            score("echarts-a", True),
            score("echarts-b island 0 (printed by chrome)", False, ["colour"], text_pass=False, worst=0.02, colour=0.4, hue=0.1, text_missing=["1,200"]),
            score("vega-a island 0 (printed by chrome)", True),
        ]
        det = {"echarts-a": {"subject": "echarts-a", "deterministic": True}}
        lines, status = compat.fixtures_section(scores, compat.fixture_meta(), det)
        # vega-a: not every comparison ran and no determinism result, so partial.
        self.assertEqual(status, {"echarts-a": "pass", "echarts-b": "fail", "vega-a": "partial"})
        self.assertEqual(compat.fixtures_section(scores, compat.fixture_meta())[1]["echarts-a"], "partial", "pass needs the determinism check")
        # Each failed check with its own number, not the shape score.
        self.assertIn("**FAIL** (colour 40.0%; text: 1 missing)", "\n".join(lines))
        self.assertIn("| echarts-a | echarts | pass (shape 0.0%) | pass (shape 0.0%) | pass (shape 0.0%) | yes |", "\n".join(lines))
        rw = "\n".join(compat.real_world_section(status, compat.fixture_meta()))
        self.assertIn("**1 failing**: echarts-b", rw)
        self.assertIn("| vega | 6.4.0 | BSD-3-Clause | required | 1 | partly run |", rw)

    def test_duplicate_fixture_scores_are_an_error(self):
        # The review's reproduction: a failing run next to a passing one.
        for run, passed in (("run-a", False), ("run-b", True)):
            d = self.root / "results" / run / "echarts-a"
            d.mkdir(parents=True)
            (d / "score.json").write_text(json.dumps(score("echarts-a island 0 (printed by chrome)", passed, [] if passed else ["shape"], worst=0.01 if passed else 0.5)))
        scores = compat.load_scores(self.root / "results")
        with self.assertRaises(compat.CompatError) as e:
            compat.fixtures_section(scores, compat.fixture_meta())
        self.assertIn("run-a", str(e.exception))
        self.assertIn("run-b", str(e.exception))
        # Either order.
        with self.assertRaises(compat.CompatError):
            compat.fixtures_section(list(reversed(scores)), compat.fixture_meta())

    def test_render_error_is_a_failure_not_not_run(self):
        failed = {"subject": "echarts-a island 0 (printed by chrome)", "pass": False, "error": "surimono: TimeLimit in island 0\nstack"}
        lines, status = compat.fixtures_section([failed], compat.fixture_meta())
        self.assertEqual(status["echarts-a"], "fail")
        self.assertIn("**FAIL** (render error: surimono: TimeLimit in island 0)", "\n".join(lines))

    def test_failed_checks_show_their_own_windows(self):
        s = score("x island 0 (printed by chrome)", False, ["shape", "hue", "size"], worst=0.25, hue=0.5)
        s["pages"][0].update({"size_match": False, "actual_size": "640x341", "expected_size": "640x340"})
        self.assertEqual(compat.cell(s), "**FAIL** (shape 25.0%; hue 50.0%; size 640x341 vs Chrome 640x340)")
        self.assertEqual(compat.cell(score("x", False, ["colour"])), "**FAIL** (colour ?)")

    def test_font_check_is_named_with_its_mismatch(self):
        # The images pass; only the font check fails (as compare.mjs writes it).
        s = score("x island 0 (printed by chrome)", True)
        s["pass"] = False
        s["font"] = {"pass": False, "compared": 3, "mismatches": [{"key": "Jan", "chrome": "400 normal", "surimono": "700 normal"}]}
        self.assertEqual(compat.cell(s), "**FAIL** (font: 1 differ (Jan: Chrome 400 normal, surimono 700 normal))")

    def test_hostile_duplicates_are_an_error(self):
        results = self.write_json(
            "hostile.json",
            {"cases": [{"id": "loop", "step": None, "outcome": "Crash"}, {"id": "loop", "step": None, "outcome": "TimeLimit"}]},
        )
        with self.assertRaises(compat.CompatError) as e:
            compat.hostile_section(results)
        self.assertIn("'loop'", str(e.exception))

    def test_hostile_results_not_in_the_manifest_fail_and_count(self):
        # The review's reproduction: a typo'd id that crashed must not vanish.
        results = self.write_json(
            "hostile.json",
            {
                "commit": "abc",
                "cases": [
                    {"id": "loop", "step": None, "outcome": "TimeLimit", "wall_ms": 2001},
                    {"id": "loop-typo", "step": None, "outcome": "Crash", "wall_ms": 3},
                    {"id": "two", "step": 3, "outcome": "Ok", "output": "c", "wall_ms": 1},
                ],
            },
        )
        text = "\n".join(compat.hostile_section(results))
        self.assertIn("1 of 7 cases end as the manifest expects (results from abc; 2 not in the manifest)", text)
        self.assertIn("| loop-typo | — | Crash (3 ms) | **FAIL**: not in the manifest |", text)
        self.assertIn("| two step 3 | — | Ok (1 ms) | **FAIL**: not in the manifest |", text)

    def test_main_exits_1_on_duplicates_and_writes_nothing(self):
        for run in ("a", "b"):
            d = self.root / "res" / run
            d.mkdir(parents=True)
            (d / "score.json").write_text(json.dumps(score("echarts-a", True)))
        out = self.root / "COMPAT.md"
        import sys
        argv = sys.argv
        sys.argv = ["compat.py", "--fixtures", str(self.root / "res"), "--out", str(out)]
        try:
            self.assertEqual(compat.main(), 1)
        finally:
            sys.argv = argv
        self.assertFalse(out.exists())

    def test_a_score_for_an_island_the_fixture_lacks_is_an_error(self):
        # Review of #28: "island 2" of a two-island fixture, failing, must not vanish.
        s = score("echarts-a island 2 (printed by chrome)", False, ["shape"], worst=0.5)
        s["_file"] = "run/echarts-a/island-2-chrome/score.json"
        with self.assertRaises(compat.CompatError) as e:
            compat.fixtures_section([s], compat.fixture_meta())
        self.assertIn("island-2", str(e.exception))
        self.assertIn("1 island", str(e.exception))

    def test_island_numbers_have_one_spelling(self):
        # Review of #28: a failing "island 01" next to a passing "island 1"
        # must not vanish. Leading zeros and non-ASCII digits are refused.
        for subject in ("echarts-a island 01 (printed by chrome)", "echarts-a island \u0660 (printed by chrome)", "echarts-a island +0 (printed by chrome)"):
            s = score(subject, False, ["shape"], worst=0.5)
            s["_file"] = "run-a/score.json"
            with self.assertRaises(compat.CompatError, msg=subject):
                compat.fixtures_section([s, score("echarts-a island 0 (printed by chrome)", True)], compat.fixture_meta())

    def test_pass_is_derived_not_trusted(self):
        s = score("x island 0 (printed by chrome)", True)
        s["text"] = {"pass": False, "missing": ["Jan"], "extra": []}
        self.assertEqual(compat.cell(s), "**FAIL** (text: 1 missing)")
        t = score("x island 0 (printed by chrome)", True)
        t["pass"] = "false"
        self.assertTrue(compat.cell(t).startswith("**FAIL**"), compat.cell(t))
        u = score("x island 0 (printed by chrome)", True)
        u["pages"][0]["pass"] = False
        self.assertTrue(compat.cell(u).startswith("**FAIL**"), compat.cell(u))

    def test_baseline_scores_never_land_in_surimonos_columns(self):
        for subject in ("echarts-a island 0", "echarts-a page", "echarts-a island 0 (printed by firefox)"):
            s = score(subject, True)
            s["_file"] = "baseline/echarts-a/fulgur/score.json"
            with self.assertRaises(compat.CompatError, msg=subject):
                compat.fixtures_section([s], compat.fixture_meta())

    def test_hostile_result_without_wall_ms_or_outcome_fails(self):
        results = self.write_json(
            "hostile.json",
            {"commit": "abc", "cases": [{"id": "loop", "step": None, "outcome": "TimeLimit"}, {"id": "req", "step": None, "wall_ms": 3}]},
        )
        text = "\n".join(compat.hostile_section(results))
        self.assertIn("| loop | TimeLimit | TimeLimit (? ms) | **FAIL**: no wall_ms, so the max_wall_ms check can't be made |", text)
        self.assertIn("| req | Exception | — (3 ms) | **FAIL**: no outcome in the results file |", text)
        self.assertIn("0 of 5 cases", text)

    def test_a_determinism_failure_fails_the_fixture(self):
        scores = [score("echarts-a island 0 (printed by chrome)", True), score("echarts-a island 0 (printed by fulgur)", True), score("echarts-a", True)]
        det = {"echarts-a": {"subject": "echarts-a", "deterministic": False, "note": "the second render's bytes differ"}}
        lines, status = compat.fixtures_section(scores, compat.fixture_meta(), det)
        self.assertEqual(status["echarts-a"], "fail")
        self.assertIn("| **FAIL** (the second render's bytes differ) |", "\n".join(lines))

    def test_determinism_files_load_and_duplicates_are_an_error(self):
        for run in ("a", "b"):
            d = self.root / "det" / run / "echarts-a"
            d.mkdir(parents=True)
            (d / "determinism.json").write_text(json.dumps({"subject": "echarts-a", "deterministic": True}))
        with self.assertRaises(compat.CompatError):
            compat.load_determinism(self.root / "det")
        self.assertEqual(list(compat.load_determinism(self.root / "det" / "a")), ["echarts-a"])

    def test_unstamped_results_never_mix_with_stamped_ones(self):
        # Review of #35: an unstamped passing score (compare.mjs by hand, maybe
        # on Chrome's own SVG) next to stamped real ones was credited to the real CLI.
        real = {"cli": "/r/target/release/surimono", "version": "surimono 0.1.0", "sha256": "2", "commit": "abc"}
        stamped = {**score("echarts-a island 0 (printed by chrome)", True), "producer": real, "_file": "a/score.json"}
        unstamped = {**score("echarts-b island 0 (printed by chrome)", True), "_file": "b/score.json"}
        with self.assertRaises(compat.CompatError) as e:
            compat.generate([stamped, unstamped], None, None, "c", None)
        self.assertIn("producer not recorded", str(e.exception))
        out = compat.generate([unstamped], None, None, "c", None)
        self.assertIn("Results: fixtures from c (producer not recorded)", out)

    def test_stand_in_and_mixed_producers_are_refused(self):
        fake = {"cli": "/r/fixtures/tools/test/fake-surimono.mjs", "version": "surimono 0.0.0 (fake-surimono.mjs: Chrome's SVGs, not surimono)", "sha256": "1", "commit": None}
        real = {"cli": "/r/target/release/surimono", "version": "surimono 0.1.0", "sha256": "2", "commit": "abc"}
        s = score("echarts-a", True)
        with self.assertRaises(compat.CompatError) as e:
            compat.generate([{**s, "producer": fake, "_file": "x/score.json"}], None, None, "c", None)
        self.assertIn("stand-in", str(e.exception))
        with self.assertRaises(compat.CompatError):
            compat.check_producers([{**s, "producer": real}, {**s, "producer": {**real, "sha256": "3"}}])
        out = compat.generate([{**s, "producer": real}], None, None, "c", None)
        self.assertIn("surimono CLI surimono 0.1.0, sha256 2, commit abc", out)

    def test_missing_inputs_say_not_run(self):
        out = compat.generate([], None, None, "c", None)
        self.assertIn("Not run yet: no surimono output", out)
        self.assertIn("the sandbox harness hasn't reported results", out)
        self.assertIn("Not run yet (`bench/run.sh`)", out)
        self.assertIn("## Known gaps", out)

    def test_bench_rows(self):
        bench = self.write_json(
            "bench.json",
            {
                "versions": {"surimono_commit": "0123456789", "chromium": "153"},
                "machine": {"ci": True},
                "method": {"runs": 20},
                "results": {
                    "grover": {"label": "Grover", "status": "measured", "summary": {"cold_ms": {"median": 2000}, "warm_ms": {"median": 900}, "cold_peak_pss_mb": {"median": 500}}},
                    "surimono-ruby": {"label": "surimono", "status": "unavailable", "reason": "gem not built\nmore"},
                },
            },
        )
        text = "\n".join(compat.bench_section(bench))
        self.assertIn("| Grover | 2,000 ms | 900 ms | 500 MB | measured |", text)
        self.assertIn("not measured: gem not built |", text)

    def test_a_result_for_a_fixture_not_in_the_checkout_is_an_error(self):
        # Review of #28: a failing result under another spelling of a fixture
        # got its own row (renderer "?") that the real-world table never counted.
        good = score("echarts-a island 0 (printed by chrome)", True)
        for name, hint in (("ECHARTS-A", "did you mean 'echarts-a'?"), ("echarts-a\u200b", "did you mean 'echarts-a'?"), ("echarts-renamed", None)):
            s = score(f"{name} island 0 (printed by chrome)", False, ["shape"], worst=0.5)
            s["_file"] = f"run/{name}/score.json"
            with self.assertRaises(compat.CompatError, msg=repr(name)) as e:
                compat.fixtures_section([good, s], compat.fixture_meta())
            self.assertIn(repr(name), str(e.exception))
            self.assertIn("not a fixture in fixtures/islands/", str(e.exception))
            if hint:
                self.assertIn(hint, str(e.exception))
        # A page subject too, and through main: exit 1, nothing written.
        d = self.root / "res" / "old-name"
        d.mkdir(parents=True)
        (d / "score.json").write_text(json.dumps(score("old-name", False, ["shape"], worst=0.5)))
        out = self.root / "COMPAT.md"
        import sys
        argv = sys.argv
        sys.argv = ["compat.py", "--fixtures", str(self.root / "res"), "--out", str(out)]
        try:
            self.assertEqual(compat.main(), 1)
        finally:
            sys.argv = argv
        self.assertFalse(out.exists())

    def test_a_score_that_compared_no_page_never_passes(self):
        # Review of #28: "pass": true with no pages read "pass (shape ?)".
        for pages in ([], None, "page 1", {"pass": True}):
            s = score("echarts-a island 0 (printed by chrome)", True)
            if pages is None:
                del s["pages"]
            else:
                s["pages"] = pages
            self.assertEqual(compat.cell(s), "**FAIL** (no pages compared)", repr(pages))
            self.assertEqual(compat.fixtures_section([s], compat.fixture_meta())[1]["echarts-a"], "fail", repr(pages))
        # A render error has no pages either; it reads as the render error alone.
        failed = {"subject": "echarts-a", "pass": False, "error": "surimono exited 3", "pages": []}
        self.assertEqual(compat.cell(failed), "**FAIL** (render error: surimono exited 3)")

    def test_nested_checks_pass_only_when_pass_is_exactly_true(self):
        # Review of #28: inside a check that is present, "false", a missing
        # pass or a value that isn't an object must fail, never pass.
        def cell_with(**changes):
            s = score("echarts-a island 0 (printed by chrome)", True)
            page = changes.pop("page", None)
            if page is not None:
                s["pages"] = [page]
            s.update(changes)
            return compat.cell(s)

        self.assertEqual(cell_with(page={"pass": "false", "failed": []}), "**FAIL** (page (shape 0.0%))")
        self.assertEqual(cell_with(page={"failed": []}), "**FAIL** (page (shape 0.0%))")
        self.assertEqual(cell_with(page="pass"), "**FAIL** (page (shape 0.0%))")
        self.assertEqual(cell_with(page={"pass": True, "failed": "shape"}), "**FAIL** (page (shape 0.0%))")
        self.assertEqual(cell_with(text={"pass": "false", "missing": [], "extra": []}), "**FAIL** (text: differs)")
        self.assertEqual(cell_with(text={}), "**FAIL** (text: differs)")
        self.assertEqual(cell_with(text="ok"), "**FAIL** (text: differs)")
        self.assertEqual(cell_with(font={"pass": 1}), "**FAIL** (font: differs)")
        self.assertEqual(cell_with(font=["pass"]), "**FAIL** (font: differs)")
        # Exactly true passes; an absent (or null) check isn't run, so it can't fail.
        self.assertEqual(cell_with(text={"pass": True, "missing": [], "extra": []}, font={"pass": True}), "pass (shape 0.0%)")
        self.assertEqual(cell_with(text=None), "pass (shape 0.0%)")

    def test_error_and_failed_are_set_as_javascript_reads_them(self):
        # Review of #50: "failed": {} and "error": {} or [] passed, while
        # lib/report.mjs (JavaScript truthiness) fails them.
        s = score("echarts-a island 0 (printed by chrome)", True)
        s["pages"] = [{"pass": True, "failed": {}}]
        self.assertEqual(compat.cell(s), "**FAIL** (page (shape 0.0%))")
        for error in ({}, [], "0", "false", 1):
            e = {**score("echarts-a island 0 (printed by chrome)", True), "error": error}
            self.assertFalse(compat.passed(e), repr(error))
            self.assertTrue(compat.cell(e).startswith("**FAIL** (render error: "), compat.cell(e))
        # Unset values aren't an error: absent, null, false, 0 and "".
        for error in (None, False, 0, 0.0, ""):
            e = {**score("echarts-a island 0 (printed by chrome)", True), "error": error}
            self.assertEqual(compat.cell(e), "pass (shape 0.0%)", repr(error))

    def test_nan_and_infinity_are_refused_on_read(self):
        # Review of #50: json.loads accepts NaN, and a NaN error read as unset
        # passed. JSON.parse (lib/report.mjs) refuses such a file outright.
        clean = '{"subject": "echarts-a", "pass": true, "worst_window": 0, "pages": [{"pass": true, "failed": []}]'
        for i, bad in enumerate(('"error": NaN', '"error": Infinity', '"error": -Infinity', '"pages": [{"pass": true, "failed": NaN}]', '"worst_window": NaN')):
            d = self.root / f"nan{i}" / "echarts-a"
            d.mkdir(parents=True)
            (d / "score.json").write_text(clean + ", " + bad + "}")
            with self.assertRaises(compat.CompatError, msg=bad) as e:
                compat.load_scores(self.root / f"nan{i}")
            self.assertIn("is not valid JSON", str(e.exception))
        # The hostile results and the bench file too.
        hostile = self.root / "hostile-nan.json"
        hostile.write_text('{"cases": [{"id": "loop", "step": null, "outcome": "TimeLimit", "wall_ms": NaN}]}')
        with self.assertRaises(compat.CompatError):
            compat.hostile_section(hostile)
        bench = self.root / "bench-nan.json"
        bench.write_text('{"results": {"grover": {"status": "measured", "summary": {"cold_ms": {"median": Infinity}}}}}')
        with self.assertRaises(compat.CompatError):
            compat.bench_section(bench)
        # A NaN that reaches failed_checks some other way counts as set, never as a pass.
        self.assertFalse(compat.passed({**score("echarts-a", True), "error": float("nan")}))
        self.assertFalse(compat.passed({**score("echarts-a", True), "pages": [{"pass": True, "failed": float("nan")}]}))

    def test_font_cell_counts_missing_and_extra_texts(self):
        # Reviews of #28 and #43: a font failure that is only missing (or
        # extra) texts read "font: 0 differ".
        s = score("x island 0 (printed by chrome)", True)
        s["pass"] = False
        s["font"] = {"pass": False, "compared": 2, "mismatches": [], "missing": [{"key": "East", "chrome": "400 normal"}], "extra": [{"key": "North", "surimono": "400 normal"}]}
        self.assertEqual(compat.cell(s), "**FAIL** (font: 1 missing, 1 extra)")
        s["font"]["mismatches"] = [{"key": "Jan", "chrome": "400 normal", "surimono": "700 normal"}]
        self.assertEqual(compat.cell(s), "**FAIL** (font: 1 differ, 1 missing, 1 extra (Jan: Chrome 400 normal, surimono 700 normal))")

    def test_a_producer_that_is_not_an_object_is_an_error(self):
        # Review of #35: "producer": "surimono 0.1.0" raised AttributeError.
        s = score("echarts-a", True)
        # Falsy values that aren't objects are refused too (#51 review); only
        # an absent, null or empty producer means "not recorded".
        for producer in ("surimono 0.1.0", ["surimono"], 3, True, "", [], 0, False):
            with self.assertRaises(compat.CompatError, msg=repr(producer)) as e:
                compat.generate([{**s, "producer": producer, "_file": "x/score.json"}], None, None, "c", None)
            self.assertIn("is not an object", str(e.exception))
        for producer in (None, {}):
            self.assertIn("(producer not recorded)", compat.generate([{**s, "producer": producer}], None, None, "c", None))
        # Values that aren't strings are compared, not hashed.
        odd = {"cli": ["/r/surimono"], "version": {"v": 1}, "sha256": "2", "commit": None}
        self.assertIn("Results: fixtures from c", compat.generate([{**s, "producer": odd}, {**s, "subject": "echarts-b", "producer": dict(odd)}], None, None, "c", None))
        with self.assertRaises(compat.CompatError):
            compat.generate([{**s, "producer": odd}, {**s, "subject": "echarts-b", "producer": {**odd, "cli": ["/other"]}}], None, None, "c", None)

    def test_malformed_result_files_are_errors(self):
        # Review of #35: a determinism.json without "subject" raised TypeError.
        for i, data in enumerate(({"deterministic": True}, {"subject": 3, "deterministic": True}, {"subject": "", "deterministic": True}, ["echarts-a"])):
            d = self.root / f"det{i}" / "echarts-a"
            d.mkdir(parents=True)
            (d / "determinism.json").write_text(json.dumps(data))
            with self.assertRaises(compat.CompatError, msg=repr(data)):
                compat.load_determinism(self.root / f"det{i}")
        d = self.root / "scores" / "echarts-a"
        d.mkdir(parents=True)
        (d / "score.json").write_text(json.dumps([score("echarts-a", True)]))
        with self.assertRaises(compat.CompatError) as e:
            compat.load_scores(self.root / "scores")
        self.assertIn("not a JSON object", str(e.exception))

    def test_determinism_files_refuse_nan_too(self):
        d = self.root / "detnan" / "echarts-a"
        d.mkdir(parents=True)
        (d / "determinism.json").write_text('{"subject": "echarts-a", "deterministic": NaN}')
        with self.assertRaises(compat.CompatError) as e:
            compat.load_determinism(self.root / "detnan")
        self.assertIn("is not valid JSON", str(e.exception))

    def test_a_determinism_result_for_an_unknown_fixture_is_an_error(self):
        scores = [score("echarts-a", True)]
        det = {"ECHARTS-A": {"subject": "ECHARTS-A", "deterministic": False, "_file": "run/ECHARTS-A/determinism.json"}}
        with self.assertRaises(compat.CompatError) as e:
            compat.fixtures_section(scores, compat.fixture_meta(), det)
        self.assertIn("a determinism result for 'ECHARTS-A'", str(e.exception))
        self.assertIn("did you mean 'echarts-a'?", str(e.exception))



class VerdictTableTest(unittest.TestCase):
    """compat.py against the shared table, and against lib/report.mjs (#51
    review: the two disagreed on some shapes)."""

    def test_shared_verdict_cases(self):
        for c in VERDICTS["cases"]:
            with self.subTest(c["name"]):
                self.assertEqual(compat.failed_checks(c["score"]), c["failed"])
                self.assertEqual(compat.passed(c["score"]), c["pass"])

    @unittest.skipUnless(shutil.which("node"), "node is not installed")
    def test_report_mjs_agrees_with_compat_on_every_combination(self):
        dims = VERDICTS["dimensions"]
        scores = []
        for values in itertools.product(*dims.values()):
            s = {"subject": "echarts-a island 0 (printed by chrome)", "worst_window": 0}
            s.update({k: v for k, v in zip(dims, values) if v != {"$absent": True}})
            scores.append(s)
        report = (HERE.parent / "lib" / "report.mjs").as_uri()
        script = (
            f"import {{ failedChecks, passes }} from {json.dumps(report)};"
            "let input = ''; process.stdin.setEncoding('utf8');"
            "process.stdin.on('data', (d) => (input += d)).on('end', () => {"
            "  process.stdout.write(JSON.stringify(JSON.parse(input).map((s) => [passes(s), failedChecks(s)])));"
            "});"
        )
        r = subprocess.run(["node", "--input-type=module", "-e", script], input=json.dumps(scores), capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stderr)
        js = json.loads(r.stdout)
        self.assertEqual(len(js), len(scores))
        differ = [(s, [compat.passed(s), compat.failed_checks(s)], j) for s, j in zip(scores, js) if [compat.passed(s), compat.failed_checks(s)] != j]
        self.assertEqual(differ[:3], [], f"{len(differ)} of {len(scores)} scores get different verdicts")


if __name__ == "__main__":
    unittest.main()
