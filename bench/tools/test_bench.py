"""Unit tests for the benchmark harness and its summary (no Chrome, no Ruby).

    python3 -I -m unittest discover -s bench/tools -p 'test_*.py'
"""

from __future__ import annotations

import os
import stat
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bench  # noqa: E402
import summary  # noqa: E402


def stats(values: list[float]) -> dict:
    return bench.stats(values)


def measured_system(label: str, cold_ms: float, warm_ms: float, cold_pss: float, warm_pss: float, n: int = 20, mem_n: int = 5) -> dict:
    r = {
        "label": label,
        "about": label,
        "done": ["cold", "cold-memory", "warm", "warm-memory"],
        "cold": [{"wall_ms": cold_ms, "cpu_ms": cold_ms} for _ in range(n)],
        "cold-memory": [{"memory": {"peak_pss_mb": cold_pss, "peak_rss_mb": cold_pss * 2, "complete": True}} for _ in range(mem_n)],
        "warm": {"iterations": [{"ms": warm_ms} for _ in range(n)]},
        "warm-memory": {"memory": {"peak_pss_mb": warm_pss, "peak_rss_mb": warm_pss * 2, "complete": True}},
    }
    return r


def info_with(results: dict, runs: int = 20, mem_runs: int = 5) -> dict:
    return {
        "versions": {},
        "machine": {"ci": True, "cpu": "test", "cores": 2, "memory_gb": 7, "loadavg_start": (0.1, 0, 0), "loadavg_end": (0.1, 0, 0)},
        "method": {
            "runs": runs,
            "memory_runs": mem_runs,
            "warmup": 2,
            "spec_runs": 20,
            "sample_interval_ms": 10,
            "report": "bench/report/report.html",
            "islands": 6,
            "spec_targets": {"speed": 2.0, "memory": 10.0},
        },
        "results": results,
    }


def comparison_line(md: str, base_label: str, label: str) -> str:
    return next(line for line in md.splitlines() if line.startswith(f"| {base_label} ÷ {label} |"))


class Ratios(unittest.TestCase):
    def test_no_ratio_against_a_grover_that_failed(self) -> None:
        # The review's reproduction: Grover fails in its 3rd cold run.
        grover = measured_system("Grover (Chrome)", 3000, 900, 600, 700)
        grover["cold"] = grover["cold"][:2]
        grover["done"] = []
        grover.update({"status": "error", "reason": "timed out", "failed_in": "cold"})
        res = {"grover": grover, "surimono-ruby": measured_system("surimono + fulgur (Ruby)", 600, 150, 60, 70)}
        bench.finalize(res, final=True, why_unfinished="budget")
        self.assertNotIn("summary", res["grover"], "a failed system's stats must not be in summary")
        self.assertEqual(res["grover"]["partial_summary"]["cold_ms"]["n"], 2)
        md = summary.write_summary(info_with(res))
        line = comparison_line(md, "Grover (Chrome)", "surimono + fulgur (Ruby)")
        self.assertIn("not measured", line)
        self.assertNotIn("met", line)
        self.assertNotIn("×", line)

    def test_no_ratio_when_the_candidate_is_incomplete(self) -> None:
        sur = measured_system("surimono + fulgur (Ruby)", 600, 150, 60, 70)
        sur["done"] = ["cold", "cold-memory"]
        del sur["warm"], sur["warm-memory"]
        res = {"grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700), "surimono-ruby": sur}
        bench.finalize(res, final=True, why_unfinished="the job's time budget ran out")
        self.assertEqual(res["surimono-ruby"]["status"], "incomplete")
        self.assertIn("time budget", res["surimono-ruby"]["reason"])
        self.assertIn("warm", res["surimono-ruby"]["reason"])
        md = summary.write_summary(info_with(res))
        self.assertNotIn("met", comparison_line(md, "Grover (Chrome)", "surimono + fulgur (Ruby)"))

    def test_met_needs_the_spec_runs(self) -> None:
        res = {
            "grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700, n=5, mem_n=2),
            "surimono-ruby": measured_system("surimono + fulgur (Ruby)", 600, 600, 60, 70, n=5, mem_n=2),
        }
        bench.finalize(res, final=True, why_unfinished="x")
        line = comparison_line(summary.write_summary(info_with(res, runs=5, mem_runs=2)), "Grover (Chrome)", "surimono + fulgur (Ruby)")
        cells = [c.strip() for c in line.strip("|").split("|")]
        self.assertEqual(cells[1], "5.0× indicative (n=5; needs 20)")
        self.assertEqual(cells[2], "1.5× indicative (n=5; needs 20)")
        self.assertEqual(cells[3], "10.0× (10.0×) **met**")  # memory runs: n equals the requested 2

    def test_met_and_not_met_with_full_runs(self) -> None:
        res = {
            "grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700),
            "surimono-ruby": measured_system("surimono + fulgur (Ruby)", 600, 600, 100, 70),
        }
        bench.finalize(res, final=True, why_unfinished="x")
        line = comparison_line(summary.write_summary(info_with(res)), "Grover (Chrome)", "surimono + fulgur (Ruby)")
        cells = [c.strip() for c in line.strip("|").split("|")]
        self.assertEqual(cells[1], "5.0× **met**")
        self.assertEqual(cells[2], "1.5× **not met**")
        self.assertEqual(cells[3], "6.0× (6.0×) **not met**")
        self.assertEqual(cells[4], "10.0× (10.0×) **met**")

    def test_incomplete_memory_gives_no_verdict(self) -> None:
        sur = measured_system("surimono + fulgur (Ruby)", 600, 150, 60, 70)
        sur["cold-memory"][0]["memory"]["complete"] = False
        res = {"grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700), "surimono-ruby": sur}
        bench.finalize(res, final=True, why_unfinished="x")
        line = comparison_line(summary.write_summary(info_with(res)), "Grover (Chrome)", "surimono + fulgur (Ruby)")
        cells = [c.strip() for c in line.strip("|").split("|")]
        self.assertIn("incomplete", cells[3])
        self.assertNotIn("met", cells[3])

    def test_fulgur_only_row_is_a_reference_point_never_a_verdict(self) -> None:
        res = {
            "grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700),
            "fulgur-prerendered": measured_system("fulgur only", 500, 100, 60, 70),
        }
        bench.finalize(res, final=True, why_unfinished="x")
        md = summary.write_summary(info_with(res))
        line = comparison_line(md, "Grover (Chrome)", "fulgur only")
        self.assertIn("reference point", line)
        self.assertNotIn("met", line)
        self.assertNotIn("lower bound", md)
        self.assertNotIn("upper bound", md)

    def test_both_grover_baselines_get_rows(self) -> None:
        res = {
            "grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700),
            "grover-shell": measured_system("Grover (chrome-headless-shell)", 2500, 800, 400, 500),
            "surimono-ruby": measured_system("surimono + fulgur (Ruby)", 600, 150, 60, 70),
        }
        bench.finalize(res, final=True, why_unfinished="x")
        md = summary.write_summary(info_with(res))
        self.assertIn("4.2×", comparison_line(md, "Grover (chrome-headless-shell)", "surimono + fulgur (Ruby)"))

    def test_measured_after_an_earlier_partial_save(self) -> None:
        res = {"grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700)}
        done = res["grover"]["done"]
        res["grover"]["done"] = ["cold"]
        bench.finalize(res, final=False, why_unfinished="x")
        self.assertEqual(res["grover"]["status"], "incomplete")
        self.assertIn("still running", res["grover"]["reason"])
        res["grover"]["done"] = done
        bench.finalize(res, final=True, why_unfinished="x")
        self.assertEqual(res["grover"]["status"], "measured")
        self.assertNotIn("reason", res["grover"])
        self.assertNotIn("partial_summary", res["grover"])
        self.assertEqual(res["grover"]["summary"]["cold_ms"]["n"], 20)

    def test_the_spec_phrase_of_the_report_shape_is_there(self) -> None:
        res = {"grover": measured_system("Grover (Chrome)", 3000, 900, 600, 700)}
        bench.finalize(res, final=True, why_unfinished="x")
        md = summary.write_summary(info_with(res))
        self.assertIn("shaped around fulgur 0.40's layout gaps", md)


class FakeProc:
    """A /proc tree of plain files: cmdline and smaps_rollup per pid."""

    def __init__(self, root: Path) -> None:
        self.root = root

    def add(self, pid: int, cmd: list[str], rss_kb: int, pss_kb: int) -> None:
        d = self.root / str(pid)
        d.mkdir(parents=True)
        (d / "cmdline").write_bytes(b"\0".join(c.encode() for c in cmd) + b"\0")
        (d / "smaps_rollup").write_text(f"00400000-7fff [rollup]\nRss:  {rss_kb} kB\nPss:  {pss_kb} kB\n")


class SamplerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.proc = FakeProc(Path(self.tmp.name))

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_keeper_is_left_out_but_its_chrome_counts(self) -> None:
        self.proc.add(10, ["ruby", "bench.rb", "grover-browser"], 50 * 1024, 40 * 1024)
        self.proc.add(11, ["node", bench.KEEPER, "/opt/chrome", "new"], 90 * 1024, 81 * 1024)
        self.proc.add(12, ["/opt/chrome", "--headless=new"], 200 * 1024, 150 * 1024)
        self.proc.add(13, ["/opt/chrome", "--type=renderer"], 100 * 1024, 60 * 1024)
        s = bench.Sampler(proc=str(self.proc.root), root=0)
        s.sample_once([10, 11, 12, 13])
        r = s.result()
        self.assertEqual(r["peak_pss_mb"], 250.0)
        self.assertEqual(r["peak_rss_mb"], 350.0)
        self.assertEqual(r["excluded"]["peak_pss_mb"], 81.0)
        self.assertEqual(r["peak_processes"], 3)
        self.assertIn("chrome (renderer)", r["processes_seen"])
        self.assertNotIn("node", r["processes_seen"])
        self.assertTrue(r["complete"])

    def test_grovers_own_node_process_counts(self) -> None:
        self.proc.add(20, ["node", "/gems/grover-1.2.10/lib/grover/js/processor.cjs"], 80 * 1024, 70 * 1024)
        s = bench.Sampler(proc=str(self.proc.root), root=0)
        s.sample_once([20])
        self.assertEqual(s.result()["peak_pss_mb"], 70.0)
        self.assertNotIn("excluded", s.result())

    def test_unreadable_process_marks_the_run_incomplete(self) -> None:
        self.proc.add(30, ["/opt/chrome", "--type=gpu-process"], 100 * 1024, 50 * 1024)
        self.proc.add(31, ["/opt/chrome", "--type=zygote"], 100 * 1024, 50 * 1024)
        real = bench._rollup

        def rollup(pid: int, proc: str = "/proc"):
            if pid == 31:
                raise PermissionError(13, "Permission denied")
            return real(pid, proc)

        bench._rollup = rollup
        try:
            s = bench.Sampler(proc=str(self.proc.root), root=0)
            s.sample_once([30, 31, 32])  # 32 is gone: not an error
        finally:
            bench._rollup = real
        r = s.result()
        self.assertFalse(r["complete"])
        self.assertEqual(r["unreadable_processes"], ["chrome (zygote)"])
        self.assertEqual(r["peak_pss_mb"], 50.0)

    def test_no_sum_of_high_water_marks(self) -> None:
        s = bench.Sampler(proc=str(self.proc.root), root=0)
        s.sample_once([])
        self.assertNotIn("sum_of_process_hwm_mb", s.result())


class RustBuild(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.bin = self.dir / "bin"
        self.bin.mkdir()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def fake_cargo(self, script: str) -> dict:
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\n" + script)
        cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
        env = dict(os.environ)
        env["PATH"] = f"{self.bin}{os.pathsep}{env['PATH']}"
        env["BENCH_RUST_TARGET_DIR"] = str(self.dir / "target")
        return env

    def test_root_path_patches_reach_bench_rust(self) -> None:
        toml = """
[workspace]
members = []
[patch.crates-io]
rquickjs-sys = { path = "vendor/rquickjs-sys" }
other = { git = "https://example.invalid/x" }
"""
        args = bench.root_patch_args(toml, Path("/repo"))
        self.assertEqual(args, ["--config", 'patch."crates-io"."rquickjs-sys".path="/repo/vendor/rquickjs-sys"'])
        self.assertEqual(bench.root_patch_args("[workspace]\n"), [])

    def test_patch_args_are_passed_to_cargo(self) -> None:
        env = self.fake_cargo(f'printf "%s\\n" "$@" > {self.dir}/args\n')
        bench.build_rust(env, self.dir, 30, min_budget_s=0)
        args = (self.dir / "args").read_text().splitlines()
        self.assertEqual(args[:2], ["build", "--release"])
        self.assertEqual(args[2:], bench.root_patch_args((bench.ROOT / "Cargo.toml").read_text()))

    def test_budget_keeps_time_for_measuring(self) -> None:
        self.assertEqual(bench.rust_build_budget({}, float("inf")), 1800)
        self.assertEqual(bench.rust_build_budget({"BENCH_RUST_BUILD_TIMEOUT": "600"}, float("inf")), 600)
        self.assertEqual(bench.rust_build_budget({}, 20 * 60), 5 * 60)

    def test_no_time_left_is_not_measured_without_building(self) -> None:
        env = self.fake_cargo(f"touch {self.dir}/ran\n")
        binary, why, status, took = bench.build_rust(env, self.dir, 60)
        self.assertIsNone(binary)
        self.assertEqual(status, "unavailable")
        self.assertIn("no time left", why)
        self.assertFalse((self.dir / "ran").exists())

    def test_timeout_kills_the_whole_build(self) -> None:
        env = self.fake_cargo(f"sleep 60 & echo $! > {self.dir}/child\nwait\n")
        t0 = time.monotonic()
        binary, why, status, took = bench.build_rust(env, self.dir, 1.5, min_budget_s=0)
        self.assertLess(time.monotonic() - t0, 20)
        self.assertIsNone(binary)
        self.assertEqual(status, "unavailable")
        self.assertIn("didn't finish", why)
        child = int((self.dir / "child").read_text())
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                state = Path(f"/proc/{child}/stat").read_text().rsplit(")", 1)[1].split()[0]
            except OSError:
                state = "gone"
            if state in ("gone", "Z", "X"):
                break
            time.sleep(0.05)
        self.assertIn(state, ("gone", "Z", "X"), "rustc-like children must die with cargo")

    def test_failed_build_is_an_error(self) -> None:
        env = self.fake_cargo("echo 'error[E0425]: cannot find value' >&2\nexit 101\n")
        binary, why, status, took = bench.build_rust(env, self.dir, 30, min_budget_s=0)
        self.assertIsNone(binary)
        self.assertEqual(status, "error")
        self.assertIn("exit 101", why)
        self.assertIn("E0425", why)


if __name__ == "__main__":
    unittest.main()
