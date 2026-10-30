#!/usr/bin/env python3
"""The Grover benchmark harness: time and peak memory, Grover against surimono.

    python3 bench/tools/bench.py --out <dir> [--runs 20] [--mem-runs 5] [--warmup 2]
                                 [--systems grover,surimono-ruby,fulgur-prerendered]

bench/run.sh installs the dependencies and calls this. Every number it writes is
measured by it; a system that can't run is reported as not measured, with the
reason, never estimated.

Systems (one row each in summary.md):

  grover              Grover 1.2.10 + Puppeteer + Chrome, charts drawn client-side.
                      cold: new Ruby process, Grover launches Chrome.
                      warm: one Ruby process and one Chrome kept running
                      (Grover's browser_ws_endpoint), the fastest Grover setup.
  grover-launch       warm only: one Ruby process, Grover's default of a new Chrome
                      per document (what a Rails app gets without extra setup).
  surimono-ruby       Surimono::Renderer#render, then the fulgur gem, in Ruby.
  fulgur-prerendered  the fulgur gem printing the report with Chrome's SVGs already
                      inlined: no JavaScript at all. A lower bound on the PDF step
                      of the surimono path, not a surimono result.

Time: wall clock per document. Cold is the whole process, from spawn to exit
(Ruby boot, setup, render, PDF written), timed here. Warm is one render inside a
long-lived process after `warmup` untimed renders, timed by the process itself.
Median and p90 (nearest rank) of `runs`.

Memory: this harness is a child subreaper, so every process a run starts,
including Chrome's helpers that detach, stays in its tree. A sampler thread
sums RSS and PSS (/proc/<pid>/smaps_rollup) over the whole tree every 10 ms;
the peak of the sum is the run's peak. PSS counts each shared page once
(Chrome's processes share most of their code), so it is the headline; summed
RSS double-counts shared pages and is reported alongside. Memory runs are
separate from timed runs, so sampling never slows a timed run.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BENCH = ROOT / "bench"
FONTS = ROOT / "fixtures" / "fonts"
TOOLS = ROOT / "fixtures" / "tools"
SAMPLE_INTERVAL_S = 0.010
SPEC_SPEED = 2.0  # docs/SPEC.md: at least 2x faster than Grover, cold and warm
SPEC_MEMORY = 10.0  # docs/SPEC.md: at least 10x lower peak memory than Grover

SYSTEMS = {
    "grover": {
        "label": "Grover (Chrome)",
        "cold": "grover",
        "warm": "grover-browser",
        "input": "grover",
        "about": "Grover 1.2.10, Puppeteer, Chrome; charts drawn client-side by ECharts/Vega in Chrome. Warm: Ruby process and Chrome reused (browser_ws_endpoint).",
    },
    "grover-launch": {
        "label": "Grover, new Chrome per document",
        "cold": None,
        "warm": "grover",
        "input": "grover",
        "about": "Warm only: Ruby process reused, Grover's default of launching Chrome for every document.",
    },
    "surimono-ruby": {
        "label": "surimono + fulgur (Ruby)",
        "cold": "surimono",
        "warm": "surimono",
        "input": "report",
        "about": "Surimono::Renderer#render (islands in QuickJS), then the fulgur gem prints the HTML.",
    },
    "fulgur-prerendered": {
        "label": "fulgur only, pre-rendered SVG (lower bound)",
        "cold": "fulgur-prerendered",
        "warm": "fulgur-prerendered",
        "input": "prerendered",
        "about": "The fulgur gem prints the report with Chrome's island SVGs already inlined. No JavaScript runs: a lower bound on the PDF step of the surimono path, NOT a surimono result.",
    },
}
DEFAULT_SYSTEMS = ["grover", "grover-launch", "surimono-ruby", "fulgur-prerendered"]


# ---------------------------------------------------------------- process tree


def become_subreaper() -> bool:
    """Orphaned descendants (Chrome's crashpad handler double-forks) are
    reparented to this process instead of init, so they stay measurable."""
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        return libc.prctl(36, 1, 0, 0, 0) == 0  # PR_SET_CHILD_SUBREAPER
    except (OSError, AttributeError):
        return False


def _children(pid: int) -> list[int]:
    kids: list[int] = []
    try:
        for tid in os.listdir(f"/proc/{pid}/task"):
            try:
                with open(f"/proc/{pid}/task/{tid}/children") as f:
                    kids.extend(int(x) for x in f.read().split())
            except OSError:
                pass
    except OSError:
        pass
    return kids


def _children_by_scan() -> dict[int, list[int]]:
    tree: dict[int, list[int]] = {}
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        try:
            with open(f"/proc/{name}/stat") as f:
                stat = f.read()
            ppid = int(stat[stat.rindex(")") + 2 :].split()[1])
            tree.setdefault(ppid, []).append(int(name))
        except (OSError, ValueError):
            pass
    return tree


HAVE_CHILDREN_FILE = os.path.exists(f"/proc/{os.getpid()}/task/{os.getpid()}/children")


def descendants(root: int) -> list[int]:
    """All live descendants of root (this harness: every process a run started)."""
    out: list[int] = []
    if HAVE_CHILDREN_FILE:
        stack = [root]
        while stack:
            for k in _children(stack.pop()):
                out.append(k)
                stack.append(k)
    else:
        tree = _children_by_scan()
        stack = [root]
        while stack:
            for k in tree.get(stack.pop(), []):
                out.append(k)
                stack.append(k)
    return out


def _rollup(pid: int) -> tuple[int, int] | None:
    """(rss_kb, pss_kb) of one process, None if it is gone or a zombie."""
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            text = f.read()
    except OSError:
        return None
    rss = re.search(r"^Rss:\s+(\d+) kB", text, re.M)
    pss = re.search(r"^Pss:\s+(\d+) kB", text, re.M)
    if not rss:
        return None
    return int(rss.group(1)), int(pss.group(1)) if pss else int(rss.group(1))


def _hwm_kb(pid: int) -> int:
    try:
        with open(f"/proc/{pid}/status") as f:
            m = re.search(r"^VmHWM:\s+(\d+) kB", f.read(), re.M)
            return int(m.group(1)) if m else 0
    except OSError:
        return 0


class Sampler(threading.Thread):
    """Samples the summed RSS and PSS of this harness's process tree."""

    def __init__(self) -> None:
        super().__init__(daemon=True)
        self.stop_event = threading.Event()
        self.samples = 0
        self.peak_rss_kb = 0
        self.peak_pss_kb = 0
        self.peak_procs = 0
        self.max_single_rss_kb = 0
        self.hwm_kb: dict[int, int] = {}
        self.names: dict[int, str] = {}
        self.t_start = 0.0
        self.t_end = 0.0

    def run(self) -> None:
        me = os.getpid()
        self.t_start = time.perf_counter()
        while not self.stop_event.is_set():
            rss_sum = pss_sum = n = 0
            for pid in descendants(me):
                r = _rollup(pid)
                if r is None:
                    continue
                rss, pss = r
                rss_sum += rss
                pss_sum += pss
                n += 1
                self.max_single_rss_kb = max(self.max_single_rss_kb, rss)
                self.hwm_kb[pid] = max(self.hwm_kb.get(pid, 0), _hwm_kb(pid))
                if pid not in self.names:
                    try:
                        with open(f"/proc/{pid}/comm") as f:
                            self.names[pid] = f.read().strip()
                    except OSError:
                        pass
            self.samples += 1
            self.peak_rss_kb = max(self.peak_rss_kb, rss_sum)
            self.peak_pss_kb = max(self.peak_pss_kb, pss_sum)
            self.peak_procs = max(self.peak_procs, n)
            self.stop_event.wait(SAMPLE_INTERVAL_S)
        self.t_end = time.perf_counter()

    def result(self) -> dict:
        span = max(self.t_end - self.t_start, 1e-9)
        return {
            "peak_pss_mb": round(self.peak_pss_kb / 1024, 1),
            "peak_rss_mb": round(self.peak_rss_kb / 1024, 1),
            "max_single_process_rss_mb": round(self.max_single_rss_kb / 1024, 1),
            "sum_of_process_hwm_mb": round(sum(self.hwm_kb.values()) / 1024, 1),
            "peak_processes": self.peak_procs,
            "processes_seen": sorted(set(self.names.values())),
            "samples": self.samples,
            "mean_sample_interval_ms": round(span * 1000 / max(self.samples, 1), 1),
        }


def reap_stragglers() -> int:
    """Kills whatever a finished run left behind and reaps it; returns how many."""
    me = os.getpid()
    left = descendants(me)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in left:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 2
        while descendants(me) and time.monotonic() < deadline:
            _reap_zombies()
            time.sleep(0.02)
        if not descendants(me):
            break
    _reap_zombies()
    return len(left)


def _reap_zombies() -> None:
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            return
        if pid == 0:
            return


# ---------------------------------------------------------------- runs


class RunError(Exception):
    def __init__(self, message: str, status: int, unavailable: bool = False):
        super().__init__(message)
        self.status = status
        self.unavailable = unavailable


def ruby_cmd(system: str, mode: str, input_arg: str, out_pdf: Path, runs: int, warmup: int) -> list[str]:
    return ["ruby", "-rbundler/setup", str(BENCH / "ruby" / "bench.rb"), system, mode, input_arg, str(out_pdf), str(runs), str(warmup)]


def run_process(cmd: list[str], env: dict, sample: bool, timeout: float) -> dict:
    cpu0 = os.times()
    sampler = Sampler() if sample else None
    t0 = time.perf_counter()
    proc = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if sampler:
        sampler.start()
    try:
        out, err = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, err = proc.communicate()
        if sampler:
            sampler.stop_event.set()
            sampler.join()
        reap_stragglers()
        raise RunError(f"timed out after {timeout:.0f} s: {' '.join(cmd)}\n{err[-2000:]}", -1)
    wall_ms = (time.perf_counter() - t0) * 1000
    if sampler:
        sampler.stop_event.set()
        sampler.join()
    stragglers = reap_stragglers()
    cpu1 = os.times()
    events = []
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    if proc.returncode != 0:
        message = next((e["message"] for e in events if e.get("event") == "error"), None) or err.strip()[-3000:] or f"exit {proc.returncode}"
        raise RunError(message, proc.returncode, unavailable=proc.returncode == 4)
    result = {
        "wall_ms": round(wall_ms, 2),
        "cpu_ms": round(((cpu1.children_user - cpu0.children_user) + (cpu1.children_system - cpu0.children_system)) * 1000, 1),
        "stragglers_killed": stragglers,
        "events": events,
    }
    if sampler:
        result["memory"] = sampler.result()
    return result


def stats(values: list[float]) -> dict:
    if not values:
        return {}
    s = sorted(values)
    p90 = s[max(0, math.ceil(0.9 * len(s)) - 1)]
    return {
        "n": len(s),
        "median": round(statistics.median(s), 1),
        "p90": round(p90, 1),
        "min": round(s[0], 1),
        "max": round(s[-1], 1),
        "mean": round(statistics.fmean(s), 1),
        "stdev": round(statistics.stdev(s), 1) if len(s) > 1 else 0.0,
    }


# ---------------------------------------------------------------- environment


def sh(cmd: list[str], cwd: Path = ROOT) -> str:
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=60).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def chrome_executable() -> str:
    if os.environ.get("BENCH_CHROME"):
        return os.environ["BENCH_CHROME"]
    path = sh(["node", "-e", "console.log(require('playwright').chromium.executablePath())"], cwd=TOOLS)
    if not path or not os.path.exists(path):
        sys.exit("Chromium not found: run npx playwright install chromium in fixtures/tools, or set BENCH_CHROME")
    return path


def lock_version(lockfile: Path, name: str) -> str:
    try:
        m = re.search(rf"^    {re.escape(name)} \(([\d.]+)", lockfile.read_text(), re.M)
        return m.group(1) if m else "unknown"
    except OSError:
        return "unknown"


def npm_version(package_dir: Path) -> str:
    try:
        return json.loads((package_dir / "package.json").read_text())["version"]
    except (OSError, KeyError, json.JSONDecodeError):
        return "not installed"


def versions(chrome: str, env: dict) -> dict:
    surimono_version = sh(
        ["ruby", "-e", "$LOAD_PATH.unshift(ENV['SURIMONO_RUBY_LIB']); begin; require 'surimono'; print Surimono::VERSION, ' (native ', (Surimono.native_version rescue '?'), ')'; rescue LoadError; print 'not built'; end"],
    ) if env.get("SURIMONO_RUBY_LIB") else "not built"
    return {
        "chromium": sh([chrome, "--version"]),
        "grover": lock_version(BENCH / "Gemfile.lock", "grover"),
        "puppeteer": npm_version(BENCH / "grover" / "node_modules" / "puppeteer"),
        "fulgur_gem": lock_version(BENCH / "Gemfile.lock", "fulgur"),
        "echarts": npm_version(TOOLS / "node_modules" / "echarts"),
        "vega": npm_version(TOOLS / "node_modules" / "vega"),
        "vega_lite": npm_version(TOOLS / "node_modules" / "vega-lite"),
        "playwright": npm_version(TOOLS / "node_modules" / "playwright"),
        "surimono_commit": sh(["git", "rev-parse", "HEAD"]),
        "surimono_describe": sh(["git", "describe", "--always", "--dirty"]),
        "surimono_gem": surimono_version,
        "ruby": sh(["ruby", "-v"]),
        "node": sh(["node", "-v"]),
        "nokogiri": lock_version(BENCH / "Gemfile.lock", "nokogiri"),
    }


def machine() -> dict:
    cpu = "unknown"
    try:
        m = re.search(r"^model name\s*:\s*(.+)$", Path("/proc/cpuinfo").read_text(), re.M)
        cpu = m.group(1) if m else cpu
        mem = re.search(r"^MemTotal:\s+(\d+) kB", Path("/proc/meminfo").read_text(), re.M)
        mem_gb = round(int(mem.group(1)) / 1024 / 1024, 1) if mem else None
    except OSError:
        mem_gb = None
    return {
        "ci": bool(os.environ.get("CI")),
        "runner": os.environ.get("RUNNER_NAME") or platform.node(),
        "os": platform.platform(),
        "cpu": cpu,
        "cores": os.cpu_count(),
        "memory_gb": mem_gb,
        "loadavg_start": os.getloadavg(),
    }


def fonts_conf(work: Path) -> Path:
    """Only fixtures/fonts reach fontconfig, for both engines (fulgur resolves
    SVG text through fontconfig; Chrome falls back through it)."""
    conf = work / "fonts.conf"
    conf.write_text(
        f"""<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>{FONTS}</dir>
  <cachedir>{work / "fontconfig-cache"}</cachedir>
</fontconfig>
"""
    )
    return conf


# ---------------------------------------------------------------- main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--work", type=Path, help="scratch dir for pages and PDFs (default <out>/work)")
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--mem-runs", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--systems", default=",".join(DEFAULT_SYSTEMS))
    args = ap.parse_args()

    out = args.out.resolve()
    work = (args.work or out / "work").resolve()
    out.mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)
    systems = [s for s in args.systems.split(",") if s]
    unknown = [s for s in systems if s not in SYSTEMS]
    if unknown:
        sys.exit(f"unknown systems: {', '.join(unknown)} (known: {', '.join(SYSTEMS)})")

    subreaper = become_subreaper()
    chrome = chrome_executable()
    env = dict(os.environ)
    env.update(
        {
            "BUNDLE_GEMFILE": str(BENCH / "Gemfile"),
            "BUNDLE_PATH": env.get("BUNDLE_PATH", str(BENCH / ".cache" / "bundle")),
            "BENCH_CHROME": chrome,
            "BENCH_FONTS_DIR": str(FONTS),
            "FONTCONFIG_FILE": str(fonts_conf(work)),
            "SURIMONO_RUBY_LIB": env.get("SURIMONO_RUBY_LIB", str(ROOT / "bindings" / "ruby" / "lib")),
        }
    )

    print("==> pages", flush=True)
    pages = json.loads(subprocess.run(["node", str(BENCH / "tools" / "pages.mjs"), str(work)], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
    inputs = {
        "grover": Path(pages["grover"]).as_uri(),
        "report": str(ROOT / "bench" / "report" / "report.html"),
        "prerendered": pages["prerendered"],
    }
    info = {
        "schema": 1,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "versions": versions(chrome, env),
        "machine": machine(),
        "method": {
            "runs": args.runs,
            "memory_runs": args.mem_runs,
            "warmup": args.warmup,
            "sample_interval_ms": SAMPLE_INTERVAL_S * 1000,
            "subreaper": subreaper,
            "report": "bench/report/report.html",
            "islands": pages["islands"],
            "spec_targets": {"speed": SPEC_SPEED, "memory": SPEC_MEMORY},
        },
    }
    results: dict[str, dict] = {s: {"label": SYSTEMS[s]["label"], "about": SYSTEMS[s]["about"]} for s in systems}
    pdf_dir = work / "pdf"
    pdf_dir.mkdir(exist_ok=True)

    def fail(system: str, phase: str, e: RunError) -> None:
        r = results[system]
        r["status"] = "unavailable" if e.unavailable else "error"
        r["reason"] = str(e).strip()
        r["failed_in"] = phase
        print(f"    {system}: {r['status']} in {phase}: {r['reason'][:400]}", flush=True)

    def live(system: str) -> bool:
        return "status" not in results[system]

    # Cold: timed runs interleaved across systems, then memory runs.
    cold = [s for s in systems if SYSTEMS[s]["cold"]]
    for phase, n, sample in (("cold", args.runs, False), ("cold-memory", args.mem_runs, True)):
        print(f"==> {phase}: {n} runs each of {', '.join(cold)}", flush=True)
        for i in range(n):
            for s in cold:
                if not live(s):
                    continue
                cfg = SYSTEMS[s]
                cmd = ruby_cmd(cfg["cold"], "cold", inputs[cfg["input"]], pdf_dir / f"{s}-cold.pdf", 1, 0)
                try:
                    r = run_process(cmd, env, sample, timeout=180)
                except RunError as e:
                    fail(s, phase, e)
                    continue
                ev = next((e for e in r["events"] if e.get("event") == "cold"), {})
                rec = {"wall_ms": r["wall_ms"], "cpu_ms": r["cpu_ms"], "setup_ms": ev.get("setup_ms"), "render_ms": ev.get("render_ms"), "phases": ev.get("phases"), "pdf_bytes": ev.get("bytes"), "stragglers_killed": r["stragglers_killed"]}
                if sample:
                    rec["memory"] = r["memory"]
                results[s].setdefault(phase, []).append(rec)
            print(f"    run {i + 1}/{n}", flush=True)

    # Warm: one timed process and one sampled process per system.
    for s in systems:
        cfg = SYSTEMS[s]
        if not cfg["warm"] or not live(s):
            continue
        for phase, sample in (("warm", False), ("warm-memory", True)):
            print(f"==> {phase}: {s}", flush=True)
            cmd = ruby_cmd(cfg["warm"], "warm", inputs[cfg["input"]], pdf_dir / f"{s}-warm.pdf", args.runs, args.warmup)
            try:
                r = run_process(cmd, env, sample, timeout=1800)
            except RunError as e:
                fail(s, phase, e)
                break
            setup = next((e for e in r["events"] if e.get("event") == "setup"), {})
            its = [e for e in r["events"] if e.get("event") == "iteration"]
            rec = {"process_wall_ms": r["wall_ms"], "process_cpu_ms": r["cpu_ms"], "setup_ms": setup.get("setup_ms"), "iterations": [{"ms": round(e["ms"], 2), "phases": e.get("phases"), "pdf_bytes": e.get("bytes")} for e in its]}
            if sample:
                rec["memory"] = r["memory"]
            results[s][phase] = rec

    # Page counts of the PDFs each system printed (a sanity check that both sides did the same work).
    for s in systems:
        for mode in ("cold", "warm"):
            pdf = pdf_dir / f"{s}-{mode}.pdf"
            if pdf.exists() and shutil.which("pdfinfo"):
                m = re.search(r"^Pages:\s+(\d+)", sh(["pdfinfo", str(pdf)]), re.M)
                if m:
                    results[s].setdefault("pdf_pages", {})[mode] = int(m.group(1))

    # Statistics.
    for s, r in results.items():
        summary = {}
        if r.get("cold"):
            summary["cold_ms"] = stats([x["wall_ms"] for x in r["cold"]])
            summary["cold_cpu_ms"] = stats([x["cpu_ms"] for x in r["cold"]])
        if r.get("cold-memory"):
            summary["cold_peak_pss_mb"] = stats([x["memory"]["peak_pss_mb"] for x in r["cold-memory"]])
            summary["cold_peak_rss_mb"] = stats([x["memory"]["peak_rss_mb"] for x in r["cold-memory"]])
        if r.get("warm"):
            summary["warm_ms"] = stats([x["ms"] for x in r["warm"]["iterations"]])
        if r.get("warm-memory"):
            summary["warm_peak_pss_mb"] = r["warm-memory"]["memory"]["peak_pss_mb"]
            summary["warm_peak_rss_mb"] = r["warm-memory"]["memory"]["peak_rss_mb"]
        r["summary"] = summary
        r.setdefault("status", "measured")

    info["machine"]["loadavg_end"] = os.getloadavg()
    info["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    info["results"] = results
    (out / "results.json").write_text(json.dumps(info, indent=2, ensure_ascii=False) + "\n")
    from summary import write_summary  # noqa: E402 (same folder)

    (out / "summary.md").write_text(write_summary(info))
    print(f"==> wrote {out / 'summary.md'} and {out / 'results.json'}", flush=True)
    errors = [s for s, r in results.items() if r["status"] == "error"]
    if errors:
        print(f"errors in: {', '.join(errors)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
