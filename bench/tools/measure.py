#!/usr/bin/env python3
"""Time and peak memory of a command, measured the way bench.py measures a cold run.

    python3 -I bench/tools/measure.py [--runs 5] [--mem-runs 5] [--interval-ms 10] [--timeout 120] -- CMD [ARG ...]
    python3 -I bench/tools/measure.py [--runs 5] ... [--warmup 0] [--offset K] --commands FILE

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

    {"wall_ms": stats, "peak_pss_mb": stats, "peak_rss_mb": stats, "max_rss_mb": stats,
     "wall_ms_runs": [...], "peak_pss_mb_runs": [...], "peak_rss_mb_runs": [...], "max_rss_mb_runs": [...],
     "memory_complete": bool, "samples_min": n, "runs": {...}}

where stats is bench.py's (n, median, p90, min, max, mean, stdev) and the
`_runs` lists are the single runs the statistics were taken from, in the order
they ran.

**Several commands compared (--commands).** FILE is a JSON list of
{"label": ..., "cmd": [...]}. Running one command's runs after the other's puts
whatever the machine did in between into the difference, so the commands take
turns: a round runs every command once, and the command that starts a round
moves on by one each round (--offset K moves the first round's start, so a
caller with many pages can change it per page). --warmup N rounds come first
and are not measured, so no command pays alone for what the first run of a
page costs (cold files). Then all timed rounds, then the sampled rounds in the
same manner. The output is

    {"commands": {label: <the object above, without "runs">, ...},
     "timed_order": [[label, ...] per round], "sampled_order": [...]}

Exits 1 when a run fails, with {"error": ..., "status": ..., "signal": ...}
(and "label" with --commands) on stdout; a command that outlives --timeout is
killed and reported the same way with "timed_out": true. Nothing is estimated.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
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
    # stderr goes to a file: a pipe nobody reads until the command has exited
    # would block a command that writes more than the pipe holds.
    with tempfile.TemporaryFile() as errors:
        t0 = time.perf_counter()
        proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=errors)
        if sampler:
            sampler.start()
        deadline = time.monotonic() + timeout
        timed_out = False
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
        if sampler:
            sampler.stop_event.set()
            sampler.join()
        stragglers = bench.reap_stragglers()
        if timed_out or proc.returncode != 0:
            code = proc.returncode
            size = errors.seek(0, os.SEEK_END)
            errors.seek(max(0, size - 3000))
            raise Failed(
                {
                    "error": f"timed out after {timeout:g} s" if timed_out else (f"killed by signal {signal.Signals(-code).name}" if code < 0 else f"exit {code}"),
                    "status": None if code < 0 else code,
                    "signal": signal.Signals(-code).name if code < 0 else None,
                    "timed_out": timed_out,
                    "stderr": errors.read().decode(errors="replace"),
                }
            )
    out = {"wall_ms": round(wall_ms, 2), "max_rss_mb": round(usage.ru_maxrss / 1024, 1), "stragglers_killed": stragglers}
    if sampler:
        out["memory"] = sampler.result()
    return out


def summary(timed: list[dict], sampled: list[dict], interval_ms: float) -> dict:
    """The statistics of one command's runs, and the single numbers they come from."""
    mems = [r["memory"] for r in sampled]
    lists = {
        "wall_ms": [r["wall_ms"] for r in timed],
        "peak_pss_mb": [m["peak_pss_mb"] for m in mems],
        "peak_rss_mb": [m["peak_rss_mb"] for m in mems],
        "max_rss_mb": [r["max_rss_mb"] for r in timed],
    }
    return {
        **{name: bench.stats(values) for name, values in lists.items()},
        **{f"{name}_runs": values for name, values in lists.items()},
        "memory_complete": all(m["complete"] for m in mems),
        "samples_min": min((m["samples"] for m in mems), default=0),
        "sample_interval_ms": interval_ms,
        "mean_sample_interval_ms": round(sum(m["mean_sample_interval_ms"] for m in mems) / len(mems), 2),
    }


def measure(cmd: list[str], runs: int, mem_runs: int, timeout: float, interval_ms: float = bench.SAMPLE_INTERVAL_S * 1000) -> dict:
    # Sampler.run reads the module's interval on every wait.
    bench.SAMPLE_INTERVAL_S = interval_ms / 1000
    timed = [run_once(cmd, False, timeout) for _ in range(runs)]
    sampled = [run_once(cmd, True, timeout) for _ in range(mem_runs)]
    return {**summary(timed, sampled, interval_ms), "runs": {"timed": timed, "sampled": sampled}}


def round_order(labels: list[str], round_index: int, offset: int = 0) -> list[str]:
    """Who runs in a round, in order: the start moves on by one each round."""
    k = (round_index + offset) % len(labels)
    return labels[k:] + labels[:k]


def measure_commands(commands: list[dict], runs: int, mem_runs: int, timeout: float, interval_ms: float, offset: int = 0, warmup: int = 0) -> dict:
    """Several commands in turns (see the module's text)."""
    bench.SAMPLE_INTERVAL_S = interval_ms / 1000
    cmds = {c["label"]: c["cmd"] for c in commands}
    labels = list(cmds)
    for i in range(warmup):
        for label in round_order(labels, i, offset):
            try:
                run_once(cmds[label], False, timeout)
            except Failed as e:
                raise Failed({**e.info, "label": label}) from None
    timed: dict[str, list[dict]] = {label: [] for label in labels}
    sampled: dict[str, list[dict]] = {label: [] for label in labels}
    orders: dict[str, list[list[str]]] = {"timed_order": [], "sampled_order": []}
    for store, sample, rounds, key in ((timed, False, runs, "timed_order"), (sampled, True, mem_runs, "sampled_order")):
        for i in range(rounds):
            order = round_order(labels, i, offset)
            orders[key].append(order)
            for label in order:
                try:
                    store[label].append(run_once(cmds[label], sample, timeout))
                except Failed as e:
                    raise Failed({**e.info, "label": label}) from None
    return {"commands": {label: summary(timed[label], sampled[label], interval_ms) for label in labels}, "warmup_rounds": warmup, **orders}


def read_commands(path: str) -> list[dict]:
    with open(path) as f:
        commands = json.load(f)
    ok = isinstance(commands, list) and commands and all(
        isinstance(c, dict) and isinstance(c.get("label"), str) and isinstance(c.get("cmd"), list) and c["cmd"] and all(isinstance(a, str) for a in c["cmd"]) for c in commands
    )
    if not ok or len({c["label"] for c in commands}) != len(commands):
        raise ValueError('--commands: expected a list of {"label": text, "cmd": [text, ...]} with different labels')
    return commands


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--mem-runs", type=int, default=5)
    ap.add_argument("--interval-ms", type=float, default=bench.SAMPLE_INTERVAL_S * 1000)
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--commands")
    ap.add_argument("--offset", type=int, default=0)
    ap.add_argument("--warmup", type=int, default=0)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    cmd = args.cmd[1:] if args.cmd[:1] == ["--"] else args.cmd
    if bool(cmd) == bool(args.commands) or args.runs < 1 or args.mem_runs < 1 or args.interval_ms <= 0 or args.timeout <= 0 or args.offset < 0 or args.warmup < 0 or (args.warmup and not args.commands):
        ap.print_usage(sys.stderr)
        return 2
    try:
        commands = read_commands(args.commands) if args.commands else None
    except (OSError, ValueError) as e:
        print(e, file=sys.stderr)
        return 2
    bench.become_subreaper()
    try:
        if commands:
            result = measure_commands(commands, args.runs, args.mem_runs, args.timeout, args.interval_ms, args.offset, args.warmup)
        else:
            result = measure(cmd, args.runs, args.mem_runs, args.timeout, args.interval_ms)
    except Failed as e:
        print(json.dumps(e.info))
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
