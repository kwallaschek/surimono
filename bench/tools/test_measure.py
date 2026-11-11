"""Unit tests for measure.py (no printer needed).

    python3 -I -m unittest discover -s bench/tools -p 'test_*.py'
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure  # noqa: E402

TOOL = str(Path(__file__).resolve().parent / "measure.py")
PY = sys.executable


def run(*args: str) -> tuple[int, dict]:
    r = subprocess.run([PY, "-I", TOOL, *args], capture_output=True, text=True, timeout=120)
    return r.returncode, (json.loads(r.stdout) if r.stdout.strip() else {})


class MeasureTest(unittest.TestCase):
    def test_measures_time_and_memory_of_a_command(self) -> None:
        # 40 MB touched and held for 0.3 s: the sampler and the kernel must both see it.
        code, out = run("--runs", "3", "--mem-runs", "2", "--", PY, "-I", "-c", "import time; x = bytearray(40_000_000); time.sleep(0.3)")
        self.assertEqual(code, 0, out)
        self.assertEqual(out["wall_ms"]["n"], 3)
        self.assertEqual(out["peak_pss_mb"]["n"], 2)
        self.assertGreaterEqual(out["wall_ms"]["median"], 300)
        self.assertLess(out["wall_ms"]["median"], 5000)
        self.assertGreaterEqual(out["peak_pss_mb"]["median"], 38)
        self.assertGreaterEqual(out["max_rss_mb"]["median"], 38)
        self.assertGreaterEqual(out["peak_rss_mb"]["median"], out["peak_pss_mb"]["median"])
        self.assertTrue(out["memory_complete"])
        self.assertGreaterEqual(out["samples_min"], 5)
        self.assertEqual(len(out["runs"]["timed"]), 3)
        # The single runs behind the statistics, so a reader can recompute them.
        self.assertEqual(out["wall_ms_runs"], [r["wall_ms"] for r in out["runs"]["timed"]])
        self.assertEqual(out["max_rss_mb_runs"], [r["max_rss_mb"] for r in out["runs"]["timed"]])
        self.assertEqual(out["peak_pss_mb_runs"], [r["memory"]["peak_pss_mb"] for r in out["runs"]["sampled"]])
        self.assertEqual(len(out["peak_rss_mb_runs"]), 2)
        self.assertNotIn("memory", out["runs"]["timed"][0])  # timed runs aren't sampled
        self.assertIn("memory", out["runs"]["sampled"][0])

    def test_a_child_of_the_command_counts(self) -> None:
        child = "import time; x = bytearray(30_000_000); time.sleep(0.3)"
        parent = f"import subprocess, sys; subprocess.run([sys.executable, '-I', '-c', {child!r}])"
        code, out = run("--runs", "1", "--mem-runs", "1", "--", PY, "-I", "-c", parent)
        self.assertEqual(code, 0, out)
        self.assertGreaterEqual(out["peak_pss_mb"]["median"], 30)
        self.assertEqual(out["runs"]["sampled"][0]["memory"]["peak_processes"], 2)

    def test_a_failing_command_is_an_error_not_a_number(self) -> None:
        code, out = run("--runs", "1", "--mem-runs", "1", "--", PY, "-I", "-c", "import sys; sys.stderr.write('no good'); sys.exit(3)")
        self.assertEqual(code, 1)
        self.assertEqual((out["error"], out["status"], out["signal"], out["timed_out"]), ("exit 3", 3, None, False))
        self.assertIn("no good", out["stderr"])
        self.assertNotIn("wall_ms", out)

    def test_a_command_killed_by_a_signal_names_it(self) -> None:
        code, out = run("--runs", "1", "--mem-runs", "1", "--", PY, "-I", "-c", "import os, signal; os.kill(os.getpid(), signal.SIGABRT)")
        self.assertEqual(code, 1)
        self.assertEqual((out["status"], out["signal"], out["timed_out"]), (None, "SIGABRT", False))

    def test_a_command_that_hangs_is_killed_and_reported(self) -> None:
        code, out = run("--runs", "1", "--mem-runs", "1", "--timeout", "0.5", "--", PY, "-I", "-c", "import time; time.sleep(60)")
        self.assertEqual(code, 1)
        self.assertTrue(out["timed_out"])
        self.assertIn("timed out", out["error"])

    def test_a_finer_interval_sees_a_short_command_more_often(self) -> None:
        cmd = ["--", PY, "-I", "-c", "import time; time.sleep(0.2)"]
        _, coarse = run("--runs", "1", "--mem-runs", "1", *cmd)
        _, fine = run("--runs", "1", "--mem-runs", "1", "--interval-ms", "1", *cmd)
        self.assertEqual(coarse["sample_interval_ms"], 10)
        self.assertEqual(fine["sample_interval_ms"], 1)
        self.assertGreater(fine["samples_min"], 2 * coarse["samples_min"])
        self.assertEqual(run("--runs", "1", "--interval-ms", "0", *cmd)[0], 2)

    def test_a_command_that_writes_a_lot_to_stderr_is_not_blocked(self) -> None:
        # 2 MB is far more than a pipe holds; read only after exit, it would hang until the timeout.
        chatty = "import sys; sys.stderr.write('x' * 2_000_000); sys.stderr.write('the end'); sys.exit({code})"
        code, out = run("--runs", "1", "--mem-runs", "1", "--timeout", "20", "--", PY, "-I", "-c", chatty.format(code=0))
        self.assertEqual(code, 0, out)
        self.assertLess(out["wall_ms"]["median"], 10_000)
        code, out = run("--runs", "1", "--mem-runs", "1", "--timeout", "20", "--", PY, "-I", "-c", chatty.format(code=4))
        self.assertEqual((code, out["error"], out["timed_out"]), (1, "exit 4", False))
        self.assertTrue(out["stderr"].endswith("the end"))
        self.assertLessEqual(len(out["stderr"]), 3000)

    def commands_file(self, commands: list) -> str:
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w") as f:
            json.dump(commands, f)
        self.addCleanup(os.unlink, path)
        return path

    def test_several_commands_take_turns(self) -> None:
        small = [PY, "-I", "-c", "import time; time.sleep(0.05)"]
        big = [PY, "-I", "-c", "import time; x = bytearray(30_000_000); time.sleep(0.15)"]
        path = self.commands_file([{"label": "small", "cmd": small}, {"label": "big", "cmd": big}])
        code, out = run("--runs", "3", "--mem-runs", "2", "--interval-ms", "1", "--commands", path)
        self.assertEqual(code, 0, out)
        self.assertEqual(out["timed_order"], [["small", "big"], ["big", "small"], ["small", "big"]])
        self.assertEqual(out["sampled_order"], [["small", "big"], ["big", "small"]])
        small_out, big_out = out["commands"]["small"], out["commands"]["big"]
        self.assertEqual((len(small_out["wall_ms_runs"]), len(big_out["wall_ms_runs"])), (3, 3))
        self.assertEqual((len(small_out["peak_pss_mb_runs"]), len(big_out["peak_pss_mb_runs"])), (2, 2))
        self.assertGreater(min(big_out["wall_ms_runs"]), min(small_out["wall_ms_runs"]) + 50)
        self.assertGreater(big_out["peak_pss_mb"]["median"], small_out["peak_pss_mb"]["median"] + 25)
        self.assertEqual(small_out["wall_ms"]["n"], 3)
        self.assertNotIn("runs", small_out)
        # --offset moves who starts the first round: a caller changes it per page.
        code, out = run("--runs", "2", "--mem-runs", "1", "--offset", "1", "--commands", path)
        self.assertEqual(code, 0, out)
        self.assertEqual(out["timed_order"], [["big", "small"], ["small", "big"]])
        self.assertEqual(out["sampled_order"], [["big", "small"]])

    def test_warmup_rounds_run_and_are_not_measured(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            # Each run appends its label to a file: every run shows, measured or not.
            log = os.path.join(d, "log")
            cmd = lambda label: [PY, "-I", "-c", f"open({log!r}, 'a').write({label!r})"]  # noqa: E731
            path = self.commands_file([{"label": "a", "cmd": cmd("a")}, {"label": "b", "cmd": cmd("b")}])
            code, out = run("--runs", "2", "--mem-runs", "1", "--warmup", "1", "--offset", "1", "--commands", path)
            self.assertEqual(code, 0, out)
            self.assertEqual(out["warmup_rounds"], 1)
            self.assertEqual(out["commands"]["a"]["wall_ms"]["n"], 2)
            # warm-up b a, timed b a, a b, sampled b a
            self.assertEqual(open(log).read(), "ba" + "baab" + "ba")
        self.assertEqual(run("--warmup", "1", "--", PY, "-c", "pass")[0], 2)

    def test_round_order(self) -> None:
        self.assertEqual([measure.round_order(["a", "b", "c"], i) for i in range(4)], [["a", "b", "c"], ["b", "c", "a"], ["c", "a", "b"], ["a", "b", "c"]])
        self.assertEqual(measure.round_order(["a", "b"], 0, 3), ["b", "a"])

    def test_a_failing_command_among_several_is_named(self) -> None:
        path = self.commands_file([{"label": "good", "cmd": [PY, "-I", "-c", "pass"]}, {"label": "bad", "cmd": [PY, "-I", "-c", "import sys; sys.exit(5)"]}])
        code, out = run("--runs", "1", "--mem-runs", "1", "--commands", path)
        self.assertEqual((code, out["label"], out["error"]), (1, "bad", "exit 5"))
        self.assertNotIn("commands", out)

    def test_commands_usage(self) -> None:
        good = self.commands_file([{"label": "a", "cmd": [PY, "-c", "pass"]}])
        self.assertEqual(run("--commands", good, "--", PY, "-c", "pass")[0], 2)  # one or the other
        for bad in ([], [{"label": "a"}], [{"label": "a", "cmd": []}], [{"label": "a", "cmd": ["x"]}, {"label": "a", "cmd": ["y"]}], {"label": "a"}):
            self.assertEqual(run("--commands", self.commands_file(bad))[0], 2, bad)
        self.assertEqual(run("--commands", good + ".missing")[0], 2)
        self.assertEqual(run("--offset", "-1", "--commands", good)[0], 2)

    def test_usage(self) -> None:
        self.assertEqual(run("--runs", "1")[0], 2)
        self.assertEqual(run("--runs", "0", "--", PY, "-c", "pass")[0], 2)

    def test_run_once_in_process(self) -> None:
        r = measure.run_once([PY, "-I", "-c", "pass"], False, 30)
        self.assertGreater(r["wall_ms"], 0)
        self.assertGreater(r["max_rss_mb"], 1)


if __name__ == "__main__":
    unittest.main()
