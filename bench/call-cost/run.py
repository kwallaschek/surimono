#!/usr/bin/env python3
"""Time per native DOM call: a getter, a one-argument method and a variadic one.

    python3 -I bench/call-cost/run.py --cli PATH [--runs 5] [--calls 1000000]

PATH is a release `surimono` CLI, built from the commit to measure (copy it out
of the shared target dir first: other builds overwrite it there). Each case is a
page whose one script makes CALLS native calls in a loop; a base page runs the
same loop with no call, and the difference over CALLS is the time per call, so
process start, fonts and the page's install cancel out. The page's clock is
virtual, so the time is the CLI's wall time, measured here. Prints the median of
RUNS runs per case, and each run, so a loaded machine shows as spread. Owned by
the conformance researcher; added by P2-DOM-21 for the call path in
crates/dom/src/glue_head.js.
"""

import argparse
import os
import statistics
import subprocess
import sys
import tempfile
import time

LOOP = "var p = document.getElementById('p'), q = document.getElementById('q'), x = 0;\n" \
       "for (let i = 0; i < {calls}; i++) {{ {body} }}\ndocument.title = String(x);"

CASES = [
    ("base (no call)", "x += i & 1;"),
    ("getter: node.nodeType", "x += p.nodeType;"),
    ("method, one argument: getAttribute('x')", "x += p.getAttribute('x').length;"),
    ("variadic: replaceChildren()", "q.replaceChildren(); x += 1;"),
]


def page(calls: int, body: str) -> str:
    script = LOOP.format(calls=calls, body=body)
    return f"<!doctype html><title></title><p id=p x=1></p><div id=q></div><script>{script}</script>"


def run_once(cli: str, font: str, path: str, out: str) -> float:
    # The output and stdout go to files in the run's temporary folder, not to
    # /dev/null, which has been a regular file on the fleet's machine (16 Nov).
    with open(out + ".stdout", "w", encoding="utf-8") as stdout:
        started = time.perf_counter()
        done = subprocess.run(
            [cli, "render", path, "-o", out, "--font", font, "--scripts",
             "--time-ms", "600000", "--pool-size", "1"],
            stdout=stdout, stderr=subprocess.PIPE, text=True)
        took = time.perf_counter() - started
    if done.returncode != 0:
        sys.exit(f"{path}: exit {done.returncode}: {done.stderr.strip()[:300]}")
    return took


def main() -> None:
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--font", default=os.path.join(here, "../../fixtures/fonts/NotoSans-Regular.ttf"))
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--calls", type=int, default=1_000_000)
    a = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        paths = []
        for i, (_, body) in enumerate(CASES):
            p = os.path.join(tmp, f"case{i}.html")
            with open(p, "w", encoding="utf-8") as f:
                f.write(page(a.calls, body))
            paths.append(p)
        times = [[] for _ in CASES]
        # Interleaved, so a load that comes and goes reaches every case alike.
        for _ in range(a.runs):
            for i, p in enumerate(paths):
                times[i].append(run_once(a.cli, a.font, p, os.path.join(tmp, "out.html")))
    base = statistics.median(times[0])
    print(f"{a.calls} calls per page, {a.runs} runs each, load average {os.getloadavg()[0]:.2f}")
    print(f"{CASES[0][0]:44} median {base:7.3f} s")
    for (name, _), t in list(zip(CASES, times))[1:]:
        m = statistics.median(t)
        per = (m - base) / a.calls * 1e9
        runs = " ".join(f"{x:.3f}" for x in t)
        print(f"{name:44} median {m:7.3f} s  {per:7.1f} ns per call  (runs: {runs})")


if __name__ == "__main__":
    main()
