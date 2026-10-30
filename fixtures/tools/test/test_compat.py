"""Tests for fixtures/tools/compat.py on a made-up repository tree.

    python3 -I -m unittest discover -s fixtures/tools/test -p 'test_*.py'
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("compat", HERE.parent / "compat.py")
compat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compat)

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


def score(subject, passed, failed=(), text_pass=None, worst=0.0):
    s = {"subject": subject, "pass": passed, "worst_window": worst, "pages": [{"pass": passed, "failed": list(failed)}]}
    if text_pass is not None:
        s["text"] = {"pass": text_pass}
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
            score("echarts-b island 0 (printed by chrome)", False, ["colour"], text_pass=False, worst=0.02),
            score("vega-a island 0 (printed by chrome)", True),
        ]
        lines, status = compat.fixtures_section(scores, compat.fixture_meta())
        self.assertEqual(status, {"echarts-a": "pass", "echarts-b": "fail", "vega-a": "partial"})
        self.assertIn("**FAIL** (colour, text; 2.0%)", "\n".join(lines))
        rw = "\n".join(compat.real_world_section(status, compat.fixture_meta()))
        self.assertIn("**1 failing**: echarts-b", rw)
        self.assertIn("| vega | 6.4.0 | BSD-3-Clause | required | 1 | partly run |", rw)

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


if __name__ == "__main__":
    unittest.main()
