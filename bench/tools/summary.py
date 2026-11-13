"""summary.md for the Grover benchmark, from the results bench.py collected.

Every number in the table is copied from results.json; nothing is estimated.
A system that couldn't run, or didn't finish every run, is listed with its
reason instead of numbers. A ratio needs both systems fully measured, and a
verdict against the SPEC target ("met" / "not met") also needs the number of
runs the SPEC asks for; anything less is "indicative".
"""

from __future__ import annotations

import statistics

BASELINES = ("grover", "grover-shell")
CANDIDATES = ("surimono-ruby", "surimono-rust", "fulgur-prerendered")
REFERENCE_POINT = "fulgur-prerendered"


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


def _n(summary: dict, key: str) -> int | None:
    v = summary.get(key)
    return v.get("n") if isinstance(v, dict) else None


def measured(r: dict | None) -> bool:
    """Fully measured: every run of every phase finished. Only these have `summary`."""
    return bool(r) and r.get("status") == "measured" and "summary" in r


def comparison_cells(g: dict, s: dict, meth: dict, reference_point: bool) -> list[str]:
    """Four cells (cold time, warm time, cold memory, warm memory) of Grover ÷ candidate.

    g and s are the `summary` of two fully measured systems. A verdict needs the
    runs the SPEC asks for on both sides (time: `spec_runs` and the requested
    runs; cold memory: the requested memory runs) and complete memory samples."""
    runs, mem_runs, spec_runs = meth["runs"], meth["memory_runs"], meth.get("spec_runs", 20)
    target_speed = meth["spec_targets"]["speed"]
    target_mem = meth["spec_targets"]["memory"]

    def verdict(x: float | None, target: float, ns: list[int | None], wanted: int, complete: bool = True, rss: float | None = None) -> str:
        if x is None:
            return "—"
        shown = _r(x) + (f" ({_r(rss)})" if rss is not None else "")
        if not complete:
            return f"{shown} **incomplete** (a process couldn't be read)"
        if reference_point:
            return f"{shown} reference point"
        if any(n != wanted for n in ns):
            return f"{shown} indicative (n={min(n or 0 for n in ns)}; needs {wanted})"
        return f"{shown} {'**met**' if x >= target else '**not met**'}"

    time_wanted = max(runs, spec_runs)
    cold_t = _ratio(_median(g.get("cold_ms")), _median(s.get("cold_ms")))
    warm_t = _ratio(_median(g.get("warm_ms")), _median(s.get("warm_ms")))
    cold_m = _ratio(_peak(g, "cold_peak_pss_mb"), _peak(s, "cold_peak_pss_mb"))
    warm_m = _ratio(g.get("warm_peak_pss_mb"), s.get("warm_peak_pss_mb"))
    cold_m_rss = _ratio(_peak(g, "cold_peak_rss_mb"), _peak(s, "cold_peak_rss_mb"))
    warm_m_rss = _ratio(g.get("warm_peak_rss_mb"), s.get("warm_peak_rss_mb"))
    cold_complete = g.get("cold_memory_complete", True) and s.get("cold_memory_complete", True)
    warm_complete = g.get("warm_memory_complete", True) and s.get("warm_memory_complete", True)
    cells = [
        verdict(cold_t, target_speed, [_n(g, "cold_ms"), _n(s, "cold_ms")], time_wanted),
        verdict(warm_t, target_speed, [_n(g, "warm_ms"), _n(s, "warm_ms")], time_wanted),
        verdict(cold_m, target_mem, [_n(g, "cold_peak_pss_mb"), _n(s, "cold_peak_pss_mb")], mem_runs, cold_complete, cold_m_rss),
        # Warm memory is one process's peak: it needs the same number of timed
        # renders as the timed runs, or a short run's lower peak would count.
        verdict(warm_m, target_mem, [g.get("warm_memory_renders"), s.get("warm_memory_renders")], time_wanted, warm_complete, warm_m_rss),
    ]
    return cells


def _not_measured(r: dict) -> str:
    status = r.get("status", "?")
    reason = r.get("reason", "").splitlines()[0][:240] if r.get("reason") else ""
    return f"**not measured** ({status} in {r.get('failed_in', '?')}): {reason}"


def write_summary(info: dict, final: bool = True) -> str:
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
    if not final:
        add("**Partial: the benchmark was still running when this was written.** Rows marked incomplete have not finished.")
        add("")
    add(
        f"Reference report: `{meth['report']}` (6 pages, {meth.get('islands', '?')} chart islands: ECharts, Vega and Vega-Lite; "
        "Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`."
    )
    add("")
    add(
        "The report is shaped around fulgur 0.40's layout gaps (`docs/COMPAT.md`): flex rows instead of `<table>`, fixed sizes "
        "instead of auto-sized flex items, one chart per page. That is right for comparing the same six pages in both engines, "
        "but it is not a typical Rails template; a page fulgur can't lay out yet isn't measured here."
    )
    add("")
    where = "GitHub Actions" if m["ci"] else "a local machine"
    load_end = m.get("loadavg_end", m["loadavg_start"])
    add(
        f"Measured on {where}: {m['cpu']}, {m['cores']} cores, {m['memory_gb']} GB, load average "
        f"{m['loadavg_start'][0]:.2f} at the start and {load_end[0]:.2f} at the end. "
        f"{meth['runs']} timed runs per cell (cold: interleaved, one process each; warm: one process), "
        f"{meth['memory_runs']} separate memory run{'s' if meth['memory_runs'] != 1 else ''} for cold and one process for warm."
    )
    if meth.get("rust_build_s") is not None:
        add("")
        add(f"The bench/rust release build took {meth['rust_build_s'] / 60:.1f} min in this run (not part of any number below).")
    if not m["ci"] and max(m["loadavg_start"][0], load_end[0]) > 0.5 * (m["cores"] or 1):
        add("")
        add("**Other work was running on this machine during the benchmark (see the load average); treat the times as indicative. The CI run is the reference.**")
    add("")

    # ------------------------------------------------------------ main table
    add("## Results")
    add("")
    add(
        "Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), "
        "median of the memory runs; summed RSS in brackets."
    )
    add("")
    add("| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |")
    add("|---|---|---|---|---|---|")
    for key, r in res.items():
        if not measured(r):
            add(f"| {r['label']} | — | — | — | — | {_not_measured(r)} |")
            continue
        s = r["summary"]
        cold = f"{_ms(s.get('cold_ms'))} ({_p90(s.get('cold_ms'))})" if s.get("cold_ms") else "n/a"
        warm = f"{_ms(s.get('warm_ms'))} ({_p90(s.get('warm_ms'))})" if s.get("warm_ms") else "n/a"
        mc = f"{_mb(_peak(s, 'cold_peak_pss_mb'))} ({_mb(_peak(s, 'cold_peak_rss_mb'))})" if s.get("cold_peak_pss_mb") else "n/a"
        mw = f"{_mb(s.get('warm_peak_pss_mb'))} ({_mb(s.get('warm_peak_rss_mb'))})" if s.get("warm_peak_pss_mb") is not None else "n/a"
        if s.get("cold_memory_complete") is False:
            mc += " **incomplete**"
        if s.get("warm_memory_complete") is False:
            mw += " **incomplete**"
        note = "measured"
        if key == REFERENCE_POINT:
            note = "measured; **reference point, not surimono** (no JavaScript runs; Chrome's SVG text is not outlined)"
        add(f"| {r['label']} | {cold} | {warm} | {mc} | {mw} | {note} |")
    add("")

    # ------------------------------------------------------------ targets
    add("## Against the SPEC targets")
    add("")
    add(
        f"`docs/SPEC.md`: surimono + fulgur at least {target_speed:.0f}× faster than Grover per document, cold and warm "
        f"(median of {meth.get('spec_runs', 20)} runs), and at least {target_mem:.0f}× lower peak memory. Ratios are Grover ÷ the other system."
    )
    add("")
    add(
        "A ratio is shown only when both systems finished every run. \"met\" or \"not met\" also needs the number of runs the SPEC asks for "
        f"on both sides ({max(meth['runs'], meth.get('spec_runs', 20))} timed runs, {meth['memory_runs']} memory run{'s' if meth['memory_runs'] != 1 else ''}); with fewer it says \"indicative\"."
    )
    add("")
    add(
        "Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per "
        "process, which inflates Grover's multi-process number; it is shown because it is the usual \"RSS of the process tree\" number, "
        "not because it is the fair one. Grover (chrome-headless-shell) is the leanest and fastest Chrome a Grover user can run; a speed or memory claim should hold "
        "against it too, or say that it doesn't."
    )
    add("")
    add("| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |")
    add("|---|---|---|---|---|")
    any_surimono = False
    for base in BASELINES:
        b = res.get(base)
        if not b:
            continue
        for key in CANDIDATES:
            r = res.get(key)
            if not r:
                continue
            name = f"{b['label']} ÷ {r['label']}"
            if not measured(b) or not measured(r):
                which = [x["label"] for x in (b, r) if not measured(x)]
                cell = f"not measured ({' and '.join(which)} not fully measured)"
                add(f"| {name} | {cell} | — | — | — |")
                continue
            if key != REFERENCE_POINT:
                any_surimono = True
            cells = comparison_cells(b["summary"], r["summary"], meth, key == REFERENCE_POINT)
            add(f"| {name} | {' | '.join(cells)} |")
    add("")
    if not any_surimono:
        add(
            "**No surimono ratio is measured yet:** the rows above say why. The fulgur-only row prints the same report with "
            "Chrome's SVGs already inlined, so it does what the surimono path does except running the chart libraries. "
            "It is a reference point, not a result: Chrome's SVGs keep `<text>`, which fulgur shapes itself, while surimono "
            "outlines text by default, so the PDF step is not the same. It is expected to bound surimono's ratios."
        )
    else:
        add(
            "The fulgur-only row is a reference point: the same report printed by fulgur without JavaScript (Chrome's SVGs, text not "
            "outlined). It is expected to bound surimono's ratios, but it is not a proven bound."
        )
    add("")

    # ------------------------------------------------------------ extras
    add("## Details")
    add("")
    add("| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |")
    add("|---|---|---|---|---|---|")
    for key, r in res.items():
        if not measured(r):
            continue
        cold = r.get("cold") or []

        def med(xs: list[float | None]) -> str:
            xs = [x for x in xs if x is not None]
            return f"{statistics.median(xs):,.0f} ms" if xs else "—"

        setup = med([c.get("setup_ms") for c in cold]) if cold else "—"
        render = med([c.get("render_ms") for c in cold]) if cold else "—"
        cpu = _ms(r["summary"].get("cold_cpu_ms"))
        warm = r.get("warm", {}).get("iterations", [])
        phases: dict[str, list] = {}
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
    excluded = [
        (r["label"], r["warm-memory"]["memory"]["excluded"])
        for r in res.values()
        if measured(r) and r.get("warm-memory", {}).get("memory", {}).get("excluded")
    ]
    for label, ex in excluded:
        add(
            f"Left out of {label}'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), "
            f"peak {_mb(ex['peak_pss_mb'])} PSS ({_mb(ex['peak_rss_mb'])} RSS). Its Chrome processes are counted."
        )
        add("")
    unreadable = sorted(
        {
            name
            for r in res.values()
            for mm in (r.get("cold-memory") or []) + ([r["warm-memory"]] if r.get("warm-memory") else [])
            for name in (mm.get("memory") or {}).get("unreadable_processes", [])
        }
    )
    if unreadable:
        add(f"**Some processes could not be read** ({', '.join(unreadable)}); the memory numbers that miss them are marked incomplete.")
        add("")

    # ------------------------------------------------------------ versions
    add("## Versions")
    add("")
    add("| | |")
    add("|---|---|")
    for k, label in [
        ("chromium", "Chromium (Grover and the pre-rendered SVGs)"),
        ("chrome_headless_shell", "chrome-headless-shell (Grover, headless shell)"),
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
        mm["memory"]["mean_sample_interval_ms"]
        for r in res.values()
        for mm in (r.get("cold-memory") or []) + ([r["warm-memory"]] if r.get("warm-memory") else [])
        if (mm.get("memory") or {}).get("mean_sample_interval_ms") is not None
    ]
    achieved = f" (achieved: {min(intervals):.0f} to {max(intervals):.0f} ms on average per run; reading many processes takes longer)" if intervals else ""
    add(
        "- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every "
        f"{meth['sample_interval_ms']:.0f} ms{achieved}; the harness is a child subreaper, "
        "so detached helpers stay counted. A peak shorter than the interval can be missed. A process that exists but can't be read "
        "marks the run's memory incomplete instead of counting as zero. `results.json` also has the largest single process."
    )
    add("- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`, so both engines see the same fonts and no system font.")
    pool = meth.get("surimono_pool_size")
    if pool:
        add(f"- **surimono's runtime pool**: {pool['size']} runtime{'s' if pool['size'] != 1 else ''} ({pool['setting']}{', ' + str(pool['cpus']) + ' CPUs here' if 'cpus' in pool else ''}). Each pooled runtime keeps its own QuickJS heap, so the pool size shows in peak memory.")
    add("")
    return "\n".join(lines) + "\n"


# ================================================================ the page set

PAGE_BASELINES = ("grover", "grover-shell")
PAGE_CANDIDATES = ("surimono-ruby", "surimono-raikiri")
PAGE_LABELS = {
    "grover": "Grover (Chrome)",
    "grover-shell": "Grover (chrome-headless-shell)",
    "surimono-ruby": "surimono + fulgur (Ruby)",
    "surimono-raikiri": "surimono + Raikiri (indicative)",
}


def _quality(r: dict) -> str:
    q = r.get("quality")
    if not q:
        return "—"
    if q.get("status") == "no output":
        return "no output"
    if q.get("status") != "scored":
        return "not scored"
    pc = q.get("page_count") or {}
    pages = "" if pc.get("chrome") == pc.get("printed") else f"; {pc.get('printed')} pages, Chrome {pc.get('chrome')}"
    return f"{'pass' if q['pass'] else '**FAIL**'} · {q['shape'] * 100:.1f}%{pages}"


def _page_ratio(x: float | None, ns: list[int | None], wanted: int, complete: bool, rss: float | None, indicative: bool, what: str) -> str:
    if x is None:
        return "—"
    shown = _r(x) + (f" ({_r(rss)})" if rss is not None else "")
    if x < 1:
        shown += " (slower)" if what == "time" else " (more memory)"
    if not complete:
        return f"{shown} **incomplete** (a process couldn't be read)"
    if indicative or any(n != wanted for n in ns):
        return f"{shown} indicative"
    return shown


def page_comparison(g: dict, s: dict, meth: dict, indicative: bool) -> dict:
    """Grover ÷ candidate on one page: four ratios, each with whether it counts
    (both sides fully measured with the runs the SPEC asks for), for the cells
    and the tallies."""
    runs, mem_runs, spec_runs = meth["runs"], meth["memory_runs"], meth.get("spec_runs", 20)
    wanted = max(runs, spec_runs)
    rows = {
        "cold_time": (_ratio(_median(g.get("cold_ms")), _median(s.get("cold_ms"))), [_n(g, "cold_ms"), _n(s, "cold_ms")], wanted, True, None, "time"),
        "warm_time": (_ratio(_median(g.get("warm_ms")), _median(s.get("warm_ms"))), [_n(g, "warm_ms"), _n(s, "warm_ms")], wanted, True, None, "time"),
        "cold_memory": (
            _ratio(_peak(g, "cold_peak_pss_mb"), _peak(s, "cold_peak_pss_mb")),
            [_n(g, "cold_peak_pss_mb"), _n(s, "cold_peak_pss_mb")],
            mem_runs,
            g.get("cold_memory_complete", True) and s.get("cold_memory_complete", True),
            _ratio(_peak(g, "cold_peak_rss_mb"), _peak(s, "cold_peak_rss_mb")),
            "memory",
        ),
        "warm_memory": (
            _ratio(g.get("warm_peak_pss_mb"), s.get("warm_peak_pss_mb")),
            [g.get("warm_memory_renders"), s.get("warm_memory_renders")],
            wanted,
            g.get("warm_memory_complete", True) and s.get("warm_memory_complete", True),
            _ratio(g.get("warm_peak_rss_mb"), s.get("warm_peak_rss_mb")),
            "memory",
        ),
    }
    out = {}
    for key, (x, ns, want, complete, rss, what) in rows.items():
        counts = x is not None and complete and not indicative and all(n == want for n in ns)
        out[key] = {"ratio": x, "counts": counts, "cell": _page_ratio(x, ns, want, complete, rss, indicative, what)}
    return out


def write_pages_summary(info: dict, final: bool = True) -> str:
    pages = info["pages"]
    v = info["versions"]
    m = info["machine"]
    meth = info["method"]
    lines: list[str] = []
    add = lines.append

    add("# Page-set benchmark")
    add("")
    if not final:
        add("**Partial: the benchmark was still running when this was written.** Rows marked incomplete have not finished.")
        add("")
    add(
        "The page path: each page's own scripts run, then the page is printed. Grover runs the scripts in Chrome and prints there; "
        "surimono runs them in QuickJS (`render_page`, printer-safe output on) and a browserless printer prints its HTML. "
        "The pages are the real-world fixture pages as they are (`fixtures/real-world`, their libraries pinned in `manifest.toml`) "
        "and long documents written by `bench/pages/gen-pages.mjs` (their sha256 in `bench/pages/SHA256SUMS`)."
    )
    add("")
    add("| Page | Kind | Title | Bytes | sha256 |")
    add("|---|---|---|---|---|")
    for p in pages:
        add(f"| {p['id']} | {p['kind']} | {p.get('title', '')} | {p.get('bytes', 0):,} | `{p.get('sha256', '')[:12]}` |")
    add("")
    where = "GitHub Actions" if m["ci"] else "a local machine"
    load_end = m.get("loadavg_end", m["loadavg_start"])
    add(
        f"Measured on {where}: {m['cpu']}, {m['cores']} cores, {m['memory_gb']} GB, load average "
        f"{m['loadavg_start'][0]:.2f} at the start and {load_end[0]:.2f} at the end. "
        f"{meth['runs']} timed runs per cell (cold: interleaved across systems, one process each; warm: one process), "
        f"{meth['memory_runs']} separate memory run{'s' if meth['memory_runs'] != 1 else ''} for cold and one process for warm, page by page."
    )
    if meth.get("raikiri_build_s") is not None:
        add("")
        add(f"The fixtures/tools/raikiri-print release build took {meth['raikiri_build_s'] / 60:.1f} min in this run (not part of any number below).")
    if not m["ci"] and max(m["loadavg_start"][0], load_end[0]) > 0.5 * (m["cores"] or 1):
        add("")
        add("**Other work was running on this machine during the benchmark (see the load average); treat the times as indicative. The CI run is the reference.**")
    add("")

    # ------------------------------------------------------------ main table
    add("## Results")
    add("")
    add(
        "Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers, and the "
        "Raikiri process, included), median of the memory runs; summed RSS in brackets. Quality: the system's cold PDF against "
        "the page's committed Chrome reference (real-world pages only), pass or **FAIL** and the shape score, by the pass rule of "
        "`fixtures/README.md`."
    )
    add("")
    add("| Page | System | Cold | Warm | Peak memory, cold | Peak memory, warm | Pages printed | Quality | Status |")
    add("|---|---|---|---|---|---|---|---|---|")
    for p in pages:
        for key, r in p["results"].items():
            pg = " / ".join(f"{k} {n}" for k, n in (r.get("pdf_pages") or {}).items()) or "—"
            q = _quality(r)
            if not measured(r):
                add(f"| {p['id']} | {r['label']} | — | — | — | — | {pg} | {q} | {_not_measured(r)} |")
                continue
            s = r["summary"]
            cold = f"{_ms(s.get('cold_ms'))} ({_p90(s.get('cold_ms'))})" if s.get("cold_ms") else "n/a"
            warm = f"{_ms(s.get('warm_ms'))} ({_p90(s.get('warm_ms'))})" if s.get("warm_ms") else "n/a"
            mc = f"{_mb(_peak(s, 'cold_peak_pss_mb'))} ({_mb(_peak(s, 'cold_peak_rss_mb'))})" if s.get("cold_peak_pss_mb") else "n/a"
            mw = f"{_mb(s.get('warm_peak_pss_mb'))} ({_mb(s.get('warm_peak_rss_mb'))})" if s.get("warm_peak_pss_mb") is not None else "n/a"
            if s.get("cold_memory_complete") is False:
                mc += " **incomplete**"
            if s.get("warm_memory_complete") is False:
                mw += " **incomplete**"
            note = "measured; **indicative system**" if key == "surimono-raikiri" else "measured"
            add(f"| {p['id']} | {r['label']} | {cold} | {warm} | {mc} | {mw} | {pg} | {q} | {note} |")
    add("")

    # ------------------------------------------------------------ ratios
    add("## Against Grover")
    add("")
    add(
        "Ratios are Grover ÷ the other system, per page: above 1× the other system is faster or uses less memory. A ratio is shown "
        f"only when both systems finished every run on that page, and it is marked indicative with fewer than {max(meth['runs'], meth.get('spec_runs', 20))} "
        f"timed runs or {meth['memory_runs']} memory runs, and always for surimono + Raikiri. `docs/SPEC.md`'s targets "
        f"({meth['spec_targets']['speed']:.0f}× time, {meth['spec_targets']['memory']:.0f}× peak PSS) are set on the islands reference report; "
        "the tallies below count against them for orientation, not as a verdict on a target."
    )
    add("")
    add("| Page | Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |")
    add("|---|---|---|---|---|---|")
    tallies: dict[tuple[str, str], dict[str, list[float]]] = {}
    for p in pages:
        res = p["results"]
        for base in PAGE_BASELINES:
            b = res.get(base)
            if not b:
                continue
            for key in PAGE_CANDIDATES:
                r = res.get(key)
                if not r:
                    continue
                name = f"{b['label']} ÷ {r['label']}"
                if not measured(b) or not measured(r):
                    which = [x["label"] for x in (b, r) if not measured(x)]
                    add(f"| {p['id']} | {name} | not measured ({' and '.join(which)} not fully measured) | — | — | — |")
                    continue
                c = page_comparison(b["summary"], r["summary"], meth, key == "surimono-raikiri")
                add(f"| {p['id']} | {name} | {c['cold_time']['cell']} | {c['warm_time']['cell']} | {c['cold_memory']['cell']} | {c['warm_memory']['cell']} |")
                t = tallies.setdefault((base, key), {k: [] for k in c})
                for k, cell in c.items():
                    if cell["counts"] or (key == "surimono-raikiri" and cell["ratio"] is not None):
                        t[k].append(cell["ratio"])
    add("")
    if tallies:
        add("### Tallies")
        add("")
        add(
            "Pages counted: those where both systems are fully measured with the runs above (for surimono + Raikiri, every page where "
            "both finished, all indicative). Range: the lowest and highest ratio over those pages."
        )
        add("")
        add("| Comparison | | Pages | Faster / less memory (> 1×) | At least the SPEC's ratio | Range |")
        add("|---|---|---|---|---|---|")
        names = {"cold_time": "cold time", "warm_time": "warm time", "cold_memory": "peak PSS, cold", "warm_memory": "peak PSS, warm"}
        for (base, key), t in tallies.items():
            b_label = PAGE_LABELS.get(base, base)
            r_label = PAGE_LABELS.get(key, key)
            for k, xs in t.items():
                target = meth["spec_targets"]["speed" if "time" in k else "memory"]
                if not xs:
                    add(f"| {b_label} ÷ {r_label} | {names[k]} | 0 | — | — | — |")
                    continue
                add(
                    f"| {b_label} ÷ {r_label} | {names[k]} | {len(xs)} | {sum(x > 1 for x in xs)} of {len(xs)} | "
                    f"{sum(x >= target for x in xs)} of {len(xs)} (≥ {target:.0f}×) | {min(xs):.1f}× to {max(xs):.1f}× |"
                )
        add("")
    quality_rows = []
    for key in ("grover", "grover-shell", "surimono-ruby", "surimono-raikiri"):
        scored = [p["results"][key]["quality"] for p in pages if key in p["results"] and (p["results"][key].get("quality") or {}).get("status") == "scored"]
        with_ref = [p for p in pages if p.get("reference") and key in p["results"]]
        if with_ref:
            label = PAGE_LABELS.get(key, key)
            quality_rows.append(f"| {label} | {sum(q['pass'] for q in scored)} of {len(with_ref)} | {len(with_ref) - len(scored)} |")
    if quality_rows:
        add("### Print quality against Chrome")
        add("")
        add("| System | Real-world pages that pass | Not scored (no PDF, or not reached) |")
        add("|---|---|---|")
        lines.extend(quality_rows)
        add("")

    # ------------------------------------------------------------ details
    add("## Details")
    add("")
    add("| Page | System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Page report (surimono) | Processes at peak |")
    add("|---|---|---|---|---|---|---|")
    for p in pages:
        for key, r in p["results"].items():
            if not measured(r):
                continue
            cold = r.get("cold") or []

            def med(xs: list[float | None]) -> str:
                xs = [x for x in xs if x is not None]
                return f"{statistics.median(xs):,.0f} ms" if xs else "—"

            setup = med([c.get("setup_ms") for c in cold]) if cold else "—"
            render = med([c.get("render_ms") for c in cold]) if cold else "—"
            cpu = _ms(r["summary"].get("cold_cpu_ms"))
            warm = r.get("warm", {}).get("iterations", [])
            phases: dict[str, list] = {}
            for it in warm:
                for k, val in (it.get("phases") or {}).items():
                    phases.setdefault(k, []).append(val)
            ph = ", ".join(f"{k.replace('_ms', '')} {med(vals)}" for k, vals in phases.items()) or "—"
            reports = [c["page"] for c in cold if c.get("page")] + [it["page"] for it in warm if it.get("page")]
            if reports:
                settled = sorted({x.get("settled") for x in reports})
                uncaught = max(x.get("uncaught", 0) for x in reports)
                failed = max(x.get("failed_count", 0) for x in reports)
                rep = f"settled {', '.join(settled)}; uncaught {uncaught}; resources failed {failed}"
            else:
                rep = "—"
            mem = (r.get("cold-memory") or [{}])[0].get("memory") or r.get("warm-memory", {}).get("memory") or {}
            add(f"| {p['id']} | {r['label']} | {setup} / {render} | {cpu} | {ph} | {rep} | {mem.get('peak_processes', '—')} |")
    add("")
    for p in pages:
        for r in p["results"].values():
            ex = measured(r) and r.get("warm-memory", {}).get("memory", {}).get("excluded")
            if ex:
                add(
                    f"Left out of {r['label']}'s warm peak on {p['id']}: the Node process that only keeps Chrome open, "
                    f"peak {_mb(ex['peak_pss_mb'])} PSS ({_mb(ex['peak_rss_mb'])} RSS). Its Chrome processes are counted."
                )
                add("")

    # ------------------------------------------------------------ versions
    add("## Versions")
    add("")
    add("| | |")
    add("|---|---|")
    for k, label in [
        ("chromium", "Chromium (Grover)"),
        ("chrome_headless_shell", "chrome-headless-shell (Grover, headless shell)"),
        ("grover", "grover gem"),
        ("puppeteer", "puppeteer"),
        ("fulgur_gem", "fulgur gem"),
        ("raikiri_print", "Raikiri printer"),
        ("surimono_gem", "surimono gem"),
        ("surimono_commit", "surimono commit"),
        ("ruby", "Ruby"),
        ("node", "Node"),
    ]:
        add(f"| {label} | {v.get(k, '—')} |")
    add("")

    # ------------------------------------------------------------ method
    add("## Method")
    add("")
    seen = {}
    for p in pages:
        for key, r in p["results"].items():
            seen.setdefault(key, r)
    for key, r in seen.items():
        add(f"- **{r['label']}**: {r['about']}")
    add(
        "- **Cold** is the wall time of a new process from spawn to exit: Ruby boot and Bundler, setup, one render, the PDF written. "
        "**Warm** is one render inside a long-lived process after " f"{meth['warmup']} untimed renders, timed by the process. "
        "Each page is measured completely (all systems, all phases) before the next."
    )
    add(
        "- **Grover** prints a file:// URL with `wait_until: load`, then waits until the page's scripts are done by the rule the "
        "Chrome references use (two animation frames and a task after load), its fastest correct setting; Grover's default for URLs, "
        "`networkidle2`, waits 500 ms after the last request. `--no-sandbox`, as in the islands benchmark."
    )
    add(
        "- **surimono** gets the page's `<script src>` and `<link href>` files as `resources:` (no network, no files otherwise), "
        "as a Rails app passes its assets, and its default limits."
    )
    intervals = [
        mm["memory"]["mean_sample_interval_ms"]
        for p in pages
        for r in p["results"].values()
        for mm in (r.get("cold-memory") or []) + ([r["warm-memory"]] if r.get("warm-memory") else [])
        if (mm.get("memory") or {}).get("mean_sample_interval_ms") is not None
    ]
    achieved = f" (achieved: {min(intervals):.0f} to {max(intervals):.0f} ms on average per run)" if intervals else ""
    add(
        "- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every "
        f"{meth['sample_interval_ms']:.0f} ms{achieved}; a peak shorter than the interval can be missed. A process that exists but "
        "can't be read marks the run's memory incomplete instead of counting as zero."
    )
    add("- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`; the printers get the same files as fonts.")
    add("- **No output**: a system that printed nothing on a page says so; how a printer fails stays out of these files.")
    pool = meth.get("surimono_pool_size")
    if pool:
        add(f"- **surimono's runtime pool**: {pool['size']} runtime{'s' if pool['size'] != 1 else ''} ({pool['setting']}{', ' + str(pool['cpus']) + ' CPUs here' if 'cpus' in pool else ''}).")
    add("")
    return "\n".join(lines) + "\n"
