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
  grover-shell        as grover, on chrome-headless-shell (Puppeteer's
                      headless: "shell"), the leanest Chrome a Grover user can run.
  grover-launch       warm only: one Ruby process, Grover's default of a new Chrome
                      per document (what a Rails app gets without extra setup).
  surimono-ruby       Surimono::Renderer#render, then the fulgur gem, in Ruby.
  surimono-rust       surimono::Renderer::render_islands, then the fulgur crate,
                      in one Rust process (bench/rust, built in release first).
  fulgur-prerendered  the fulgur gem printing the report with Chrome's SVGs already
                      inlined: no JavaScript at all. A reference point for the PDF
                      step, not a surimono result (Chrome's SVGs keep <text>, which
                      fulgur shapes itself; surimono outlines text by default).

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
separate from timed runs, so sampling never slows a timed run. The Node process
that only keeps warm Grover's Chrome open (bench/grover/browser.cjs) is left out
of the sum and reported on its own; its Chrome is counted. A process that can't
be read marks the run's memory incomplete; it is never counted as zero.

Results: results.json and summary.md are rewritten after every phase, so a run
cut short (a slow build, the CI job's time limit) still publishes what it
measured. A system is "measured" only when every run of every phase finished;
otherwise its numbers are kept apart as partial_summary and no ratio uses them.
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
SPEC_RUNS = 20  # docs/SPEC.md: median of 20 runs
# The Node process that keeps warm Grover's Chrome open. It only holds Chrome;
# a lean deployment starts Chrome itself, so it isn't Grover's cost.
KEEPER = str(BENCH / "grover" / "browser.cjs")
# Time kept for measuring when the bench/rust build shares a CI job's budget.
MEASURE_RESERVE_S = 15 * 60

SYSTEMS = {
    "grover": {
        "label": "Grover (Chrome)",
        "cold": "grover",
        "warm": "grover-browser",
        "input": "grover",
        "about": "Grover 1.2.10, Puppeteer, Chrome; charts drawn client-side by ECharts/Vega in Chrome. Warm: Ruby process and Chrome reused (browser_ws_endpoint); the Node process that only keeps Chrome open is left out of the memory sum (reported separately).",
    },
    "grover-shell": {
        "label": "Grover (chrome-headless-shell)",
        "cold": "grover-shell",
        "warm": "grover-shell-browser",
        "input": "grover",
        "about": "As Grover (Chrome), on chrome-headless-shell of the same build (Puppeteer's headless: \"shell\"), the leanest Chrome a Grover user can run.",
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
    "surimono-rust": {
        "label": "surimono + fulgur (Rust)",
        "cold": "rust",
        "warm": "rust",
        "input": "report",
        "about": "surimono::Renderer::render_islands, then the fulgur crate 0.40.0, in one Rust process (bench/rust, release build).",
    },
    "fulgur-prerendered": {
        "label": "fulgur only, Chrome's SVGs (reference point)",
        "cold": "fulgur-prerendered",
        "warm": "fulgur-prerendered",
        "input": "prerendered",
        "about": "The fulgur gem prints the report with Chrome's island SVGs already inlined. No JavaScript runs. Chrome's SVGs keep <text>, which fulgur shapes itself, while surimono outlines text by default, so the PDF step is not identical: a reference point, expected to bound surimono's ratios, NOT a surimono result.",
    },
}
# Order of the cold rotation, the warm phases and the tables: the systems the
# SPEC targets need come first, so a run cut short still has them.
DEFAULT_SYSTEMS = ["grover", "surimono-ruby", "surimono-rust", "fulgur-prerendered", "grover-shell", "grover-launch"]
BASELINES = ("grover", "grover-shell")


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


def _rollup(pid: int, proc: str = "/proc") -> tuple[int, int] | None:
    """(rss_kb, pss_kb) of one process, None if it is gone or a zombie.
    Raises PermissionError (EACCES, EPERM) when it exists but can't be read:
    the caller must not count that as zero."""
    try:
        with open(f"{proc}/{pid}/smaps_rollup") as f:
            text = f.read()
    except PermissionError:
        raise
    except OSError:
        return None
    rss = re.search(r"^Rss:\s+(\d+) kB", text, re.M)
    pss = re.search(r"^Pss:\s+(\d+) kB", text, re.M)
    if not rss:
        return None
    return int(rss.group(1)), int(pss.group(1)) if pss else int(rss.group(1))


def _cmdline(pid: int, proc: str = "/proc") -> str:
    try:
        with open(f"{proc}/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode(errors="replace").strip()
    except OSError:
        return ""


def _name(cmd: str, pid: int | None = None, proc: str = "/proc") -> str:
    """Short process name: argv[0]'s basename, plus Chrome's --type=; the
    kernel's comm when the command line is empty (an exiting process)."""
    if not cmd:
        try:
            with open(f"{proc}/{pid}/comm") as f:
                return f.read().strip() or "?"
        except OSError:
            return "?"
    name = os.path.basename(cmd.split(" ", 1)[0])
    m = re.search(r"--type=([\w-]+)", cmd)
    return f"{name} ({m.group(1)})" if m else name


class Sampler(threading.Thread):
    """Samples the summed RSS and PSS of this harness's process tree.

    Processes whose command line contains `exclude` (warm Grover's keeper) are
    summed separately, not into the peak; their children still count. The
    command line is read on every sample, because a process sampled between
    fork and exec still shows its parent's."""

    def __init__(self, exclude: str | None = KEEPER, proc: str = "/proc", root: int | None = None) -> None:
        super().__init__(daemon=True)
        self.exclude = exclude
        self.proc = proc
        self.root = root if root is not None else os.getpid()
        self.stop_event = threading.Event()
        self.samples = 0
        self.peak_rss_kb = 0
        self.peak_pss_kb = 0
        self.peak_procs = 0
        self.max_single_rss_kb = 0
        self.excluded_peak_rss_kb = 0
        self.excluded_peak_pss_kb = 0
        self.excluded_seen = False
        self.names: set[str] = set()
        self.unreadable: dict[int, str] = {}
        self.t_start = 0.0
        self.t_end = 0.0

    def sample_once(self, pids: list[int]) -> None:
        rss_sum = pss_sum = ex_rss = ex_pss = n = 0
        for pid in pids:
            cmd = _cmdline(pid, self.proc)
            try:
                r = _rollup(pid, self.proc)
            except PermissionError:
                self.unreadable[pid] = _name(cmd, pid, self.proc)
                continue
            if r is None:
                continue
            rss, pss = r
            if self.exclude and self.exclude in cmd:
                self.excluded_seen = True
                ex_rss += rss
                ex_pss += pss
                continue
            rss_sum += rss
            pss_sum += pss
            n += 1
            self.max_single_rss_kb = max(self.max_single_rss_kb, rss)
            self.names.add(_name(cmd, pid, self.proc))
        self.samples += 1
        self.peak_rss_kb = max(self.peak_rss_kb, rss_sum)
        self.peak_pss_kb = max(self.peak_pss_kb, pss_sum)
        self.peak_procs = max(self.peak_procs, n)
        self.excluded_peak_rss_kb = max(self.excluded_peak_rss_kb, ex_rss)
        self.excluded_peak_pss_kb = max(self.excluded_peak_pss_kb, ex_pss)

    def run(self) -> None:
        self.t_start = time.perf_counter()
        while not self.stop_event.is_set():
            self.sample_once(descendants(self.root))
            self.stop_event.wait(SAMPLE_INTERVAL_S)
        self.t_end = time.perf_counter()

    def result(self) -> dict:
        span = max(self.t_end - self.t_start, 1e-9)
        out = {
            "peak_pss_mb": round(self.peak_pss_kb / 1024, 1),
            "peak_rss_mb": round(self.peak_rss_kb / 1024, 1),
            "max_single_process_rss_mb": round(self.max_single_rss_kb / 1024, 1),
            "peak_processes": self.peak_procs,
            "processes_seen": sorted(self.names),
            "samples": self.samples,
            "mean_sample_interval_ms": round(span * 1000 / max(self.samples, 1), 1),
            "complete": not self.unreadable,
            "unreadable_processes": sorted(set(self.unreadable.values())),
        }
        if self.excluded_seen:
            out["excluded"] = {
                "what": "the Node process that only keeps Chrome open (bench/grover/browser.cjs); its Chrome is counted",
                "peak_pss_mb": round(self.excluded_peak_pss_kb / 1024, 1),
                "peak_rss_mb": round(self.excluded_peak_rss_kb / 1024, 1),
            }
        return out


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


RUST_BINARY: Path | None = None


def ruby_cmd(system: str, mode: str, input_arg: str, out_pdf: Path, runs: int, warmup: int) -> list[str]:
    if system == "rust":
        return [str(RUST_BINARY), mode, input_arg, str(out_pdf), str(runs), str(warmup)]
    return ["ruby", "-rbundler/setup", str(BENCH / "ruby" / "bench.rb"), system, mode, input_arg, str(out_pdf), str(runs), str(warmup)]


def rust_build_budget(env: dict, remaining_s: float) -> float:
    """Seconds the bench/rust build may take: BENCH_RUST_BUILD_TIMEOUT (default
    30 min), and never so long that the measurements no longer fit the job."""
    budget = float(env.get("BENCH_RUST_BUILD_TIMEOUT") or 1800)
    return min(budget, remaining_s - MEASURE_RESERVE_S)


def root_patch_args(cargo_toml: str, root: Path = ROOT) -> list[str]:
    """`--config` arguments that give bench/rust (its own workspace) the root
    workspace's path patches, e.g. the vendored rquickjs-sys: a [patch] only
    applies in the workspace that declares it, and surimono-core links only
    against the patched engine."""
    import tomllib

    args: list[str] = []
    for registry, entries in (tomllib.loads(cargo_toml).get("patch") or {}).items():
        for name, spec in entries.items():
            if isinstance(spec, dict) and "path" in spec:
                path = (root / spec["path"]).resolve().as_posix()
                args += ["--config", f"patch.{json.dumps(registry)}.{json.dumps(name)}.path={json.dumps(path)}"]
    return args


def build_rust(env: dict, work: Path, budget_s: float, min_budget_s: float = 120) -> tuple[Path | None, str, str, float | None]:
    """Builds bench/rust in release within budget_s seconds.

    Returns (binary, "", "ok", seconds) or (None, why, status, seconds), where
    status is "unavailable" (the facade can't be built against yet, or the
    build didn't fit its time budget: not measured, not a failure) or "error"
    (the build failed: a bench bug, the job goes red)."""
    lib = (ROOT / "crates" / "surimono" / "src" / "lib.rs").read_text()
    if "Renderer" not in lib:
        return None, "the facade on this commit has no surimono::Renderer yet (P1-PKG-02), so bench/rust can't build", "unavailable", None
    if budget_s < min_budget_s:
        return None, f"no time left in the job's budget to build bench/rust ({max(budget_s, 0) / 60:.1f} min after keeping {MEASURE_RESERVE_S // 60} min for measuring)", "unavailable", None
    target = Path(env.get("BENCH_RUST_TARGET_DIR", str(BENCH / ".cache" / "target")))
    benv = dict(env)
    benv["CARGO_TARGET_DIR"] = str(target)
    log = work / "rust-build.log"
    print(f"==> building bench/rust (release, target {target}, budget {budget_s / 60:.0f} min; log {log})", flush=True)
    t0 = time.monotonic()
    with log.open("w") as f:
        # cwd bench/rust so rustup uses bench/rust/rust-toolchain.toml (fulgur's krilla needs rustc 1.89).
        # Its own session, so a timeout kills cargo and every rustc it started.
        cmd = ["cargo", "build", "--release", *root_patch_args((ROOT / "Cargo.toml").read_text())]
        print(f"    {' '.join(cmd)}", flush=True)
        proc = subprocess.Popen(cmd, cwd=BENCH / "rust", env=benv, stdout=f, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = proc.wait(timeout=budget_s)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait()
            reap_stragglers()
            took = time.monotonic() - t0
            return None, (
                f"the bench/rust release build didn't finish within its {budget_s / 60:.0f}-minute budget "
                "(BENCH_RUST_BUILD_TIMEOUT, or what the job had left); what it compiled stays in the cache for the next run"
            ), "unavailable", round(took, 1)
    took = time.monotonic() - t0
    if code != 0:
        tail = "\n".join(log.read_text(errors="replace").splitlines()[-25:])
        return None, f"bench/rust did not build (exit {code}):\n{tail}", "error", round(took, 1)
    return target / "release" / "surimono-bench", "", "ok", round(took, 1)


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


def sh(cmd: list[str], cwd: Path = ROOT, env: dict | None = None) -> str:
    try:
        return subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=60).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def pool_size_note(env: dict) -> dict:
    """surimono's runtime pool size in this run: BENCH_POOL_SIZE if set (both
    drivers pass it to the renderer), else the facade's default, which is the
    number of CPUs clamped to 1..8 (crates/surimono/src/backend.rs)."""
    if env.get("BENCH_POOL_SIZE"):
        return {"setting": "BENCH_POOL_SIZE", "size": int(env["BENCH_POOL_SIZE"])}
    try:
        cpus = len(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        cpus = os.cpu_count() or 1
    return {"setting": "default (CPUs clamped to 1..8)", "size": max(1, min(cpus, 8)), "cpus": cpus}


def chrome_executable() -> str:
    if os.environ.get("BENCH_CHROME"):
        return os.environ["BENCH_CHROME"]
    path = sh(["node", "-e", "console.log(require('playwright').chromium.executablePath())"], cwd=TOOLS)
    if not path or not os.path.exists(path):
        sys.exit("Chromium not found: run npx playwright install chromium in fixtures/tools, or set BENCH_CHROME")
    return path


def shell_executable(chrome: str) -> str | None:
    """chrome-headless-shell of the same Playwright build as `chrome`
    (<browsers>/chromium-N/chrome-linux64/chrome next to
    <browsers>/chromium_headless_shell-N/chrome-headless-shell-linux64/chrome-headless-shell),
    or BENCH_CHROME_SHELL. None if it isn't installed."""
    if os.environ.get("BENCH_CHROME_SHELL"):
        return os.environ["BENCH_CHROME_SHELL"]
    p = Path(chrome)
    m = re.fullmatch(r"chromium-(\d+)", p.parent.parent.name)
    if not m:
        return None
    shell = p.parent.parent.parent / f"chromium_headless_shell-{m.group(1)}" / "chrome-headless-shell-linux64" / "chrome-headless-shell"
    return str(shell) if shell.exists() else None


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
    # The probe gets the benchmark's own environment (SURIMONO_RUBY_LIB); any
    # failure is reported with its class, never as an empty cell.
    probe = (
        "begin; $LOAD_PATH.unshift(ENV.fetch('SURIMONO_RUBY_LIB')); require 'surimono'; "
        "print Surimono::VERSION, ' (native ', (Surimono.native_version rescue '?'), ')'; "
        "rescue Exception => e; print 'not built (', e.class, ': ', e.message.lines.first.to_s.strip, ')'; end"
    )
    surimono_version = (sh(["ruby", "-e", probe], env=env) or "not built (the probe printed nothing)") if env.get("SURIMONO_RUBY_LIB") else "not built"
    return {
        "chromium": sh([chrome, "--version"]),
        "chrome_headless_shell": sh([env["BENCH_CHROME_SHELL"], "--version"]) if env.get("BENCH_CHROME_SHELL") else "not installed",
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


# ---------------------------------------------------------------- statistics


def phases_of(system: str) -> list[str]:
    cfg = SYSTEMS[system]
    return (["cold", "cold-memory"] if cfg["cold"] else []) + (["warm", "warm-memory"] if cfg["warm"] else [])


def summarize(r: dict) -> dict:
    """Statistics of whatever runs r holds."""
    summary: dict = {}
    if r.get("cold"):
        summary["cold_ms"] = stats([x["wall_ms"] for x in r["cold"]])
        summary["cold_cpu_ms"] = stats([x["cpu_ms"] for x in r["cold"]])
    if r.get("cold-memory"):
        mems = [x["memory"] for x in r["cold-memory"]]
        summary["cold_peak_pss_mb"] = stats([m["peak_pss_mb"] for m in mems])
        summary["cold_peak_rss_mb"] = stats([m["peak_rss_mb"] for m in mems])
        summary["cold_memory_complete"] = all(m.get("complete", True) for m in mems)
    if r.get("warm"):
        summary["warm_ms"] = stats([x["ms"] for x in r["warm"]["iterations"]])
    if r.get("warm-memory"):
        mem = r["warm-memory"]["memory"]
        summary["warm_peak_pss_mb"] = mem["peak_pss_mb"]
        summary["warm_peak_rss_mb"] = mem["peak_rss_mb"]
        # The peak of one process that rendered this many timed documents.
        summary["warm_memory_renders"] = len(r["warm-memory"].get("iterations", []))
        summary["warm_memory_complete"] = mem.get("complete", True)
    return summary


def finalize(results: dict, final: bool, why_unfinished: str) -> None:
    """Sets each system's status and statistics. Only a system whose every phase
    finished is "measured" and gets `summary`; anything else keeps its numbers
    as `partial_summary`, which no ratio or verdict uses."""
    for s, r in results.items():
        if r.get("status") in ("error", "unavailable"):
            pass
        elif all(p in r.get("done", []) for p in phases_of(s)):
            r["status"] = "measured"
            r.pop("reason", None)
            r.pop("failed_in", None)
        else:
            r["status"] = "incomplete"
            missing = [p for p in phases_of(s) if p not in r.get("done", [])]
            r["reason"] = (why_unfinished if final else "still running") + f" (not finished: {', '.join(missing)})"
            r["failed_in"] = missing[0]
        stats_ = summarize(r)
        if r["status"] == "measured":
            r["summary"] = stats_
            r.pop("partial_summary", None)
        else:
            r.pop("summary", None)
            if stats_:
                r["partial_summary"] = stats_


# ---------------------------------------------------------------- main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--work", type=Path, help="scratch dir for pages and PDFs (default <out>/work)")
    ap.add_argument("--runs", type=int, default=SPEC_RUNS)
    ap.add_argument("--mem-runs", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--systems", default=",".join(DEFAULT_SYSTEMS))
    ap.add_argument(
        "--time-budget-min",
        type=float,
        default=float(os.environ.get("BENCH_TIME_BUDGET_MIN") or 0) or None,
        help="stop starting new work this many minutes after BENCH_START_EPOCH (or now); "
        "what is left is reported as not measured (bench/run.sh sets it in CI)",
    )
    args = ap.parse_args()

    out = args.out.resolve()
    work = (args.work or out / "work").resolve()
    out.mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)
    systems = [s for s in args.systems.split(",") if s]
    unknown = [s for s in systems if s not in SYSTEMS]
    if unknown:
        sys.exit(f"unknown systems: {', '.join(unknown)} (known: {', '.join(SYSTEMS)})")
    start_epoch = float(os.environ.get("BENCH_START_EPOCH") or time.time())
    deadline = start_epoch + args.time_budget_min * 60 if args.time_budget_min else None

    def remaining() -> float:
        return deadline - time.time() if deadline else math.inf

    subreaper = become_subreaper()
    chrome = chrome_executable()
    shell = shell_executable(chrome)
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
    if shell:
        env["BENCH_CHROME_SHELL"] = shell

    results: dict[str, dict] = {s: {"label": SYSTEMS[s]["label"], "about": SYSTEMS[s]["about"], "done": []} for s in systems}
    info = {
        "schema": 2,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "versions": versions(chrome, env),
        "machine": machine(),
        "method": {
            "runs": args.runs,
            "memory_runs": args.mem_runs,
            "warmup": args.warmup,
            "spec_runs": SPEC_RUNS,
            "sample_interval_ms": SAMPLE_INTERVAL_S * 1000,
            "subreaper": subreaper,
            "report": "bench/report/report.html",
            "spec_targets": {"speed": SPEC_SPEED, "memory": SPEC_MEMORY},
            "time_budget_min": args.time_budget_min,
            "surimono_pool_size": pool_size_note(env),
        },
        "results": results,
    }
    why_unfinished = "the job's time budget ran out"

    def save(final: bool = False) -> None:
        finalize(results, final, why_unfinished)
        info["machine"]["loadavg_end"] = os.getloadavg()
        info["finished_at" if final else "saved_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        (out / "results.json").write_text(json.dumps(info, indent=2, ensure_ascii=False) + "\n")
        from summary import write_summary  # noqa: E402 (same folder)

        (out / "summary.md").write_text(write_summary(info, final=final))

    def fail(system: str, phase: str, e: RunError) -> None:
        r = results[system]
        r["status"] = "unavailable" if e.unavailable else "error"
        r["reason"] = str(e).strip()
        r["failed_in"] = phase
        print(f"    {system}: {r['status']} in {phase}: {r['reason'][:400]}", flush=True)

    def live(system: str) -> bool:
        return results[system].get("status") not in ("error", "unavailable")

    if "surimono-ruby" in results and env.get("BENCH_GEM_BUILD_FAILED"):
        # run.sh tried to build the gem (the checkout has a Surimono::Renderer)
        # and the build failed: a bug, not a system that can't run yet.
        results["surimono-ruby"].update({"status": "error", "reason": "the surimono gem did not build (bundle exec rake compile in bindings/ruby; see the job log)", "failed_in": "build"})
    if "grover-shell" in results and not shell:
        results["grover-shell"].update({"status": "unavailable", "reason": "chrome-headless-shell is not installed next to Playwright's Chromium (or set BENCH_CHROME_SHELL)", "failed_in": "setup"})

    if "surimono-rust" in systems:
        global RUST_BINARY
        RUST_BINARY, why, status, took = build_rust(env, work, rust_build_budget(env, remaining()))
        info["method"]["rust_build_s"] = took
        if RUST_BINARY is None:
            print(f"    surimono-rust: {status}: {why.splitlines()[0]}", flush=True)
            results["surimono-rust"].update({"status": status, "reason": why, "failed_in": "build"})
        else:
            print(f"    bench/rust built in {took / 60:.1f} min", flush=True)

    print("==> pages", flush=True)
    pages = json.loads(subprocess.run(["node", str(BENCH / "tools" / "pages.mjs"), str(work)], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
    inputs = {
        "grover": Path(pages["grover"]).as_uri(),
        "report": str(ROOT / "bench" / "report" / "report.html"),
        "prerendered": pages["prerendered"],
    }
    info["method"]["islands"] = pages["islands"]
    pdf_dir = work / "pdf"
    pdf_dir.mkdir(exist_ok=True)
    save()

    out_of_time = False

    def time_left(need_s: float) -> bool:
        nonlocal out_of_time
        if remaining() < need_s:
            if not out_of_time:
                print(f"==> stopping: {remaining() / 60:.1f} min left of the job's time budget", flush=True)
            out_of_time = True
        return not out_of_time

    # Cold: timed runs interleaved across systems, then memory runs.
    cold = [s for s in systems if SYSTEMS[s]["cold"]]
    for phase, n, sample in (("cold", args.runs, False), ("cold-memory", args.mem_runs, True)):
        print(f"==> {phase}: {n} runs each of {', '.join(s for s in cold if live(s))}", flush=True)
        for i in range(n):
            if not time_left(60):
                break
            for s in cold:
                if not live(s):
                    continue
                cfg = SYSTEMS[s]
                cmd = ruby_cmd(cfg["cold"], "cold", inputs[cfg["input"]], pdf_dir / f"{s}-cold.pdf", 1, 0)
                try:
                    r = run_process(cmd, env, sample, timeout=min(180, max(remaining(), 1)))
                except RunError as e:
                    fail(s, phase, e)
                    continue
                ev = next((e for e in r["events"] if e.get("event") == "cold"), {})
                rec = {"wall_ms": r["wall_ms"], "cpu_ms": r["cpu_ms"], "setup_ms": ev.get("setup_ms"), "render_ms": ev.get("render_ms"), "phases": ev.get("phases"), "pdf_bytes": ev.get("bytes"), "stragglers_killed": r["stragglers_killed"]}
                if sample:
                    rec["memory"] = r["memory"]
                results[s].setdefault(phase, []).append(rec)
            print(f"    run {i + 1}/{n}", flush=True)
        for s in cold:
            if live(s) and len(results[s].get(phase, [])) == n:
                results[s]["done"].append(phase)
        save()

    # Warm: one timed process and one sampled process per system. A render gets
    # 30 s on average (Grover with a new Chrome per document takes about 3).
    warm_timeout = 30 * (args.runs + args.warmup) + 120
    for s in systems:
        cfg = SYSTEMS[s]
        if not cfg["warm"] or not live(s):
            continue
        for phase, sample in (("warm", False), ("warm-memory", True)):
            if not time_left(120):
                break
            print(f"==> {phase}: {s}", flush=True)
            cmd = ruby_cmd(cfg["warm"], "warm", inputs[cfg["input"]], pdf_dir / f"{s}-warm.pdf", args.runs, args.warmup)
            try:
                r = run_process(cmd, env, sample, timeout=min(warm_timeout, max(remaining(), 1)))
            except RunError as e:
                fail(s, phase, e)
                break
            setup = next((e for e in r["events"] if e.get("event") == "setup"), {})
            its = [e for e in r["events"] if e.get("event") == "iteration"]
            rec = {"process_wall_ms": r["wall_ms"], "process_cpu_ms": r["cpu_ms"], "setup_ms": setup.get("setup_ms"), "iterations": [{"ms": round(e["ms"], 2), "phases": e.get("phases"), "pdf_bytes": e.get("bytes")} for e in its]}
            if sample:
                rec["memory"] = r["memory"]
            results[s][phase] = rec
            if len(its) == args.runs:
                results[s]["done"].append(phase)
            save()

    # Page counts of the PDFs each system printed (a sanity check that both sides did the same work).
    for s in systems:
        for mode in ("cold", "warm"):
            pdf = pdf_dir / f"{s}-{mode}.pdf"
            if pdf.exists() and shutil.which("pdfinfo"):
                m = re.search(r"^Pages:\s+(\d+)", sh(["pdfinfo", str(pdf)]), re.M)
                if m:
                    results[s].setdefault("pdf_pages", {})[mode] = int(m.group(1))

    save(final=True)
    print(f"==> wrote {out / 'summary.md'} and {out / 'results.json'}", flush=True)
    for s, r in results.items():
        if r["status"] != "measured":
            print(f"    {s}: {r['status']}: {r.get('reason', '').splitlines()[0][:300] if r.get('reason') else ''}", flush=True)
    errors = [s for s, r in results.items() if r["status"] == "error"]
    if errors:
        print(f"errors in: {', '.join(errors)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
