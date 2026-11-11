"""Unit tests for measure.py (no printer needed).

    python3 -I -m unittest discover -s bench/tools -p 'test_*.py'
"""

from __future__ import annotations

import json
import subprocess
import sys
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

    def test_usage(self) -> None:
        self.assertEqual(run("--runs", "1")[0], 2)
        self.assertEqual(run("--runs", "0", "--", PY, "-c", "pass")[0], 2)

    def test_run_once_in_process(self) -> None:
        r = measure.run_once([PY, "-I", "-c", "pass"], False, 30)
        self.assertGreater(r["wall_ms"], 0)
        self.assertGreater(r["max_rss_mb"], 1)


if __name__ == "__main__":
    unittest.main()
