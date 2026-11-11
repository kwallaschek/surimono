#!/usr/bin/env python3
"""Time and peak memory of one command, measured the way bench.py measures a cold run.

    python3 -I bench/tools/measure.py [--runs 5] [--mem-runs 5] [--interval-ms 10] [--timeout 120] -- CMD [ARG ...]

For a printer on one page (fixtures/tools/printers.mjs): wall clock from spawn
to exit over `runs` unsampled runs, then `mem-runs` runs under bench.py's
Sampler, which sums PSS and RSS (/proc/<pid>/smaps_rollup) over the command's
whole process tree; the peak of the sum is that run's peak. Timed and sampled
runs are separate, so sampling never slows a timed run.

The sampler's interval is bench.py's 10 ms unless --interval-ms says otherwise.
A print takes about 50 ms, and at 10 ms the sampler saw it 3 to 8 times and
read up to 11 MB under the kernel's figure, so the printer matrix samples every
millisecond. `samples_min` says how often the shortest sampled run was seen,
and the kernel's own peak RSS of the command (wait4's ru_maxrss, from the timed
runs) is reported next to the sampled peaks: it has no sampling gap, but it is
RSS of one process, not summed PSS.

The command runs in the caller's working directory and environment. Prints one
JSON object on stdout:

    {"wall_ms": stats, "peak_pss_mb": stats, "peak_rss_mb": stats,
     "max_rss_mb": stats, "memory_complete": bool, "samples_min": n, "runs": [...]}

where stats is bench.py's (n, median, p90, min, max, mean, stdev). Exits 1 when
a run fails, with {"error": ..., "status": ..., "signal": ...} on stdout; a
command that outlives --timeout is killed and reported the same way with
"timed_out": true. Nothing is estimated.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bench  # noqa: E402


class Failed(Exception):
    def __init__(self, info: dict):
        super().__init__(info.get("error", "failed"))
        self.info = info


def run_once(cmd: list[str], sample: bool, timeout: float) -> dict:
    """One run of cmd: wall time and the kernel's peak RSS; with `sample`, the
    sampler's peaks over the whole tree too."""
    sampler = bench.Sampler(exclude=None) if sample else None
    t0 = time.perf_counter()
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if sampler:
        sampler.start()
    deadline = time.monotonic() + timeout
    timed_out = False
    status = None
    usage = None
    while True:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            break
        if time.monotonic() > deadline:
            timed_out = True
            proc.kill()
            _, status, usage = os.wait4(proc.pid, 0)
            break
        time.sleep(0.0005)
    wall_ms = (time.perf_counter() - t0) * 1000
    proc.returncode = os.waitstatus_to_exitcode(status)
    err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
    if proc.stderr:
        proc.stderr.close()
    if sampler:
        sampler.stop_event.set()
        sampler.join()
    stragglers = bench.reap_stragglers()
    if timed_out or proc.returncode != 0:
        code = proc.returncode
        raise Failed(
            {
                "error": f"timed out after {timeout:.0f} s" if timed_out else (f"killed by signal {signal.Signals(-code).name}" if code < 0 else f"exit {code}"),
                "status": None if code < 0 else code,
                "signal": signal.Signals(-code).name if code < 0 else None,
                "timed_out": timed_out,
                "stderr": err[-3000:],
            }
        )
    out = {"wall_ms": round(wall_ms, 2), "max_rss_mb": round(usage.ru_maxrss / 1024, 1), "stragglers_killed": stragglers}
    if sampler:
        out["memory"] = sampler.result()
    return out


def measure(cmd: list[str], runs: int, mem_runs: int, timeout: float, interval_ms: float = bench.SAMPLE_INTERVAL_S * 1000) -> dict:
    # Sampler.run reads the module's interval on every wait.
    bench.SAMPLE_INTERVAL_S = interval_ms / 1000
    timed = [run_once(cmd, False, timeout) for _ in range(runs)]
    sampled = [run_once(cmd, True, timeout) for _ in range(mem_runs)]
    mems = [r["memory"] for r in sampled]
    return {
        "wall_ms": bench.stats([r["wall_ms"] for r in timed]),
        "peak_pss_mb": bench.stats([m["peak_pss_mb"] for m in mems]),
        "peak_rss_mb": bench.stats([m["peak_rss_mb"] for m in mems]),
        "max_rss_mb": bench.stats([r["max_rss_mb"] for r in timed]),
        "memory_complete": all(m["complete"] for m in mems),
        "samples_min": min((m["samples"] for m in mems), default=0),
        "sample_interval_ms": interval_ms,
        "mean_sample_interval_ms": round(sum(m["mean_sample_interval_ms"] for m in mems) / len(mems), 2),
        "runs": {"timed": timed, "sampled": sampled},
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--mem-runs", type=int, default=5)
    ap.add_argument("--interval-ms", type=float, default=bench.SAMPLE_INTERVAL_S * 1000)
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    cmd = args.cmd[1:] if args.cmd[:1] == ["--"] else args.cmd
    if not cmd or args.runs < 1 or args.mem_runs < 1 or args.interval_ms <= 0:
        ap.print_usage(sys.stderr)
        return 2
    bench.become_subreaper()
    try:
        result = measure(cmd, args.runs, args.mem_runs, args.timeout, args.interval_ms)
    except Failed as e:
        print(json.dumps(e.info))
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
