"""summary.md for the Grover benchmark, from the results bench.py collected.

Every number in the table is copied from results.json; nothing is estimated.
A system that couldn't run is listed with its reason instead of numbers.
"""

from __future__ import annotations

import statistics


def _ms(s: dict | None) -> str:
    return f"{s['median']:,.0f} ms" if s else "—"


def _p90(s: dict | None) -> str:
    return f"{s['p90']:,.0f} ms" if s else "—"


def _mb(x: float | None) -> str:
    return f"{x:,.0f} MB" if x is not None else "—"


def _ratio(a: float | None, b: float | None) -> float | None:
    if a is None or b is None or b == 0:
        return None
    return a / b


def _r(x: float | None) -> str:
    return f"{x:.1f}×" if x is not None else "—"


def _median(s: dict | None) -> float | None:
    return s["median"] if s else None


def _peak(summary: dict, key: str) -> float | None:
    v = summary.get(key)
    if isinstance(v, dict):
        return v.get("median")
    return v


def write_summary(info: dict) -> str:
    res = info["results"]
    v = info["versions"]
    m = info["machine"]
    meth = info["method"]
    target_speed = meth["spec_targets"]["speed"]
    target_mem = meth["spec_targets"]["memory"]
    lines: list[str] = []
    add = lines.append

    add("# Grover benchmark")
    add("")
    add(
        f"Reference report: `{meth['report']}` (6 pages, {meth['islands']} chart islands: ECharts, Vega and Vega-Lite; "
        "Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`."
    )
    add("")
    where = "GitHub Actions" if m["ci"] else "a local machine"
    add(
        f"Measured on {where}: {m['cpu']}, {m['cores']} cores, {m['memory_gb']} GB, load average "
        f"{m['loadavg_start'][0]:.2f} at the start and {m['loadavg_end'][0]:.2f} at the end. "
        f"{meth['runs']} timed runs per cell (cold: interleaved, one process each; warm: one process), "
        f"{meth['memory_runs']} separate memory runs for cold and one for warm."
    )
    if not m["ci"] and max(m["loadavg_start"][0], m["loadavg_end"][0]) > 0.5 * (m["cores"] or 1):
        add("")
        add("**Other work was running on this machine during the benchmark (see the load average); treat the times as indicative. The CI run is the reference.**")
    add("")

    # ------------------------------------------------------------ main table
    add("## Results")
    add("")
    add("Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), median of the memory runs; summed RSS in brackets.")
    add("")
    add("| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |")
    add("|---|---|---|---|---|---|")
    for key, r in res.items():
        s = r.get("summary", {})
        status = r.get("status", "measured")
        if status != "measured":
            reason = r.get("reason", "").splitlines()[0][:220] if r.get("reason") else ""
            add(f"| {r['label']} | — | — | — | — | **not measured** ({status} in {r.get('failed_in', '?')}): {reason} |")
            continue
        cold = f"{_ms(s.get('cold_ms'))} ({_p90(s.get('cold_ms'))})" if s.get("cold_ms") else "n/a"
        warm = f"{_ms(s.get('warm_ms'))} ({_p90(s.get('warm_ms'))})" if s.get("warm_ms") else "n/a"
        mc = f"{_mb(_peak(s, 'cold_peak_pss_mb'))} ({_mb(_peak(s, 'cold_peak_rss_mb'))})" if s.get("cold_peak_pss_mb") else "n/a"
        mw = f"{_mb(s.get('warm_peak_pss_mb'))} ({_mb(s.get('warm_peak_rss_mb'))})" if s.get("warm_peak_pss_mb") else "n/a"
        note = "measured"
        if key == "fulgur-prerendered":
            note = "measured; **lower bound only, not surimono** (no JavaScript runs)"
        add(f"| {r['label']} | {cold} | {warm} | {mc} | {mw} | {note} |")
    add("")

    # ------------------------------------------------------------ targets
    g = res.get("grover", {}).get("summary", {})
    add("## Against the SPEC targets")
    add("")
    add(f"`docs/SPEC.md`: surimono + fulgur at least {target_speed:.0f}× faster than Grover per document, cold and warm (median), and at least {target_mem:.0f}× lower peak memory. Ratios are Grover ÷ the other system.")
    add("")
    add(
        "Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per "
        "process, which inflates Grover's multi-process number; it is shown because it is the usual \"RSS of the process tree\" number, "
        "not because it is the fair one."
    )
    add("")
    add("| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |")
    add("|---|---|---|---|---|")
    any_surimono = False
    for key in ("surimono-ruby", "surimono-rust", "fulgur-prerendered"):
        r = res.get(key)
        if not r:
            continue
        s = r.get("summary", {})
        if r.get("status") != "measured" or not g:
            add(f"| Grover ÷ {r['label']} | not measured | not measured | not measured | not measured |")
            continue
        ratios = [
            _ratio(_median(g.get("cold_ms")), _median(s.get("cold_ms"))),
            _ratio(_median(g.get("warm_ms")), _median(s.get("warm_ms"))),
            _ratio(_peak(g, "cold_peak_pss_mb"), _peak(s, "cold_peak_pss_mb")),
            _ratio(g.get("warm_peak_pss_mb"), s.get("warm_peak_pss_mb")),
        ]
        rss = [
            _ratio(_peak(g, "cold_peak_rss_mb"), _peak(s, "cold_peak_rss_mb")),
            _ratio(g.get("warm_peak_rss_mb"), s.get("warm_peak_rss_mb")),
        ]
        targets = [target_speed, target_speed, target_mem, target_mem]
        if key == "fulgur-prerendered":
            cells = [f"{_r(x)} (bound)" for x in ratios]
        else:
            any_surimono = True
            cells = [f"{_r(x)} {'**met**' if x is not None and x >= t else '**not met**'}" if x is not None else "—" for x, t in zip(ratios, targets)]
        cells[2] += f" ({_r(rss[0])})"
        cells[3] += f" ({_r(rss[1])})"
        add(f"| Grover ÷ {r['label']} | {' | '.join(cells)} |")
    add("")
    if not any_surimono:
        add(
            "**No surimono number is measured yet:** the surimono rows above say why. The fulgur-only row prints the same report with "
            "Chrome's SVGs already inlined, so it does everything the surimono path does except running the chart libraries. "
            "Its ratios are therefore an upper bound on what surimono + fulgur can reach, not a result."
        )
        add("")
    else:
        add("The fulgur-only row is an upper bound on the surimono ratios (the same PDF work without running any JavaScript).")
        add("")

    # ------------------------------------------------------------ extras
    add("## Details")
    add("")
    add("| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |")
    add("|---|---|---|---|---|---|")
    for key, r in res.items():
        if r.get("status") != "measured":
            continue
        cold = r.get("cold") or []
        def med(xs: list[float]) -> str:
            xs = [x for x in xs if x is not None]
            return f"{statistics.median(xs):,.0f} ms" if xs else "—"
        setup = med([c.get("setup_ms") for c in cold]) if cold else "—"
        render = med([c.get("render_ms") for c in cold]) if cold else "—"
        cpu = _ms(r["summary"].get("cold_cpu_ms"))
        warm = r.get("warm", {}).get("iterations", [])
        phases = {}
        for it in warm:
            for k, val in (it.get("phases") or {}).items():
                phases.setdefault(k, []).append(val)
        ph = ", ".join(f"{k.replace('_ms', '')} {med(vals)}" for k, vals in phases.items()) or "—"
        pages = r.get("pdf_pages", {})
        pg = " / ".join(f"{k} {n}" for k, n in pages.items()) or "—"
        mem = (r.get("cold-memory") or [{}])[0].get("memory") or r.get("warm-memory", {}).get("memory") or {}
        procs = mem.get("peak_processes", "—")
        add(f"| {r['label']} | {setup} / {render} | {cpu} | {ph} | {pg} | {procs} |")
    add("")

    # ------------------------------------------------------------ versions
    add("## Versions")
    add("")
    add("| | |")
    add("|---|---|")
    for k, label in [
        ("chromium", "Chromium (Grover and the pre-rendered SVGs)"),
        ("grover", "grover gem"),
        ("puppeteer", "puppeteer"),
        ("fulgur_gem", "fulgur gem"),
        ("surimono_gem", "surimono gem"),
        ("surimono_commit", "surimono commit"),
        ("echarts", "echarts"),
        ("vega", "vega"),
        ("vega_lite", "vega-lite"),
        ("ruby", "Ruby"),
        ("node", "Node"),
    ]:
        add(f"| {label} | {v.get(k, '—')} |")
    add("")

    # ------------------------------------------------------------ method
    add("## Method")
    add("")
    for key, r in res.items():
        add(f"- **{r['label']}**: {r['about']}")
    add(
        "- **Cold** is the wall time of a new process from spawn to exit: Ruby boot and Bundler, setup, one render, the PDF written. "
        "The OS page cache is warm after the first run, so cold means a new process, not a cold disk."
    )
    add("- **Warm** is one render inside a long-lived process after " f"{meth['warmup']} untimed renders, timed by the process (monotonic clock); the PDF write is outside the timing.")
    add(
        "- **Grover** prints a file:// URL with `wait_until: load` and waits for the islands' done flag. Grover's default for URLs is "
        "`networkidle2`, which waits 500 ms after the last request on every document, so Grover is measured at its fastest correct setting. "
        "Chrome runs with `--no-sandbox` (root and Ubuntu 24.04 runners need it). Charts are drawn client-side by the same ECharts, Vega and "
        "Vega-Lite builds and the same island contract as the Chrome references (`fixtures/tools/lib/islands-browser.js`)."
    )
    intervals = [
        m["memory"]["mean_sample_interval_ms"]
        for r in res.values()
        for m in (r.get("cold-memory") or []) + ([r["warm-memory"]] if r.get("warm-memory") else [])
        if m.get("memory")
    ]
    achieved = f" (achieved: {min(intervals):.0f} to {max(intervals):.0f} ms on average per run; reading many processes takes longer)" if intervals else ""
    add(
        "- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every "
        f"{meth['sample_interval_ms']:.0f} ms{achieved}; the harness is a child subreaper, "
        "so detached helpers stay counted. A peak shorter than the interval can be missed; `results.json` also has each process's own high-water "
        "mark summed (an upper bound) and the largest single process."
    )
    add("- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`, so both engines see the same fonts and no system font.")
    add("")
    return "\n".join(lines) + "\n"
