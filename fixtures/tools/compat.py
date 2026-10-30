#!/usr/bin/env python3
"""Generates docs/COMPAT.md from machine-readable test results.

    python3 -I fixtures/tools/compat.py [--fixtures DIR] [--hostile FILE] [--bench FILE]
                                        [--commit SHA] [--out docs/COMPAT.md] [--check]

Inputs (each optional; a missing one makes its section say "not run"):

  --fixtures DIR  any directory tree holding score.json files written by
                  fixtures/tools/compare.sh (page mode and island mode, either
                  printer). The fixture is the first word of each file's
                  "subject"; fixtures/islands/*/meta.toml names their renderers.
  --hostile FILE  the hostile-suite results (format below). Pass or fail is
                  decided here, against fixtures/hostile/manifest.toml, not
                  taken from the file.
  --bench FILE    bench/run.sh's results.json.

The real-world section comes from fixtures/real-world/manifest.toml plus the
fixture results of each library's renderer.

Hand-written sections survive regeneration: everything between the lines
<!-- hand-written: begin --> and <!-- hand-written: end --> in the existing
--out file is copied into the new one unchanged. --check exits 1 when the
generated file differs from --out (nothing is written).

Hostile results format (schema 1), one entry per case and step:

    {"schema": 1, "commit": "<sha>",
     "cases": [{"id": "infinite-loop", "step": null, "outcome": "TimeLimit",
                "output": null, "message": "...", "wall_ms": 2001}, ...]}

outcome is one of MemoryLimit, TimeLimit, StackOverflow, Exception, Ok, or
Crash and Hang (the harness saw the process die or exceed max_wall_ms). step
is 1, 2, ... for `steps` cases, null otherwise.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BEGIN = "<!-- hand-written: begin -->"
END = "<!-- hand-written: end -->"

DEFAULT_HAND_WRITTEN = """## Known gaps

None recorded yet.
"""


# ---------------------------------------------------------------- inputs


def load_scores(directory: Path | None) -> list[dict]:
    if not directory:
        return []
    scores = []
    for f in sorted(directory.rglob("score.json")):
        try:
            data = json.loads(f.read_text())
        except (OSError, json.JSONDecodeError) as e:
            raise SystemExit(f"{f}: {e}")
        data["_file"] = str(f)
        scores.append(data)
    return scores


def classify(score: dict) -> tuple[str, str]:
    """(fixture, comparison) where comparison is page, island-N or island-N-fulgur."""
    subject = score.get("subject", "")
    fixture = subject.split(" ", 1)[0]
    m = re.search(r"island (\d+)(?: \(printed by (\w+)\))?", subject)
    if m:
        kind = f"island-{m.group(1)}" + ("-fulgur" if m.group(2) == "fulgur" else "")
    else:
        kind = "page"
    return fixture, kind


def fixture_meta() -> dict[str, dict]:
    meta = {}
    for f in sorted((ROOT / "fixtures" / "islands").glob("*/meta.toml")):
        meta[f.parent.name] = tomllib.loads(f.read_text(encoding="utf-8"))
    return meta


def failed_checks(score: dict) -> list[str]:
    """The checks a score failed, by name (from compare.mjs's per-page "failed" lists)."""
    names: list[str] = []
    for p in score.get("pages", []):
        for n in p.get("failed") or ([] if p.get("pass") else ["page"]):
            if n not in names:
                names.append(n)
    if score.get("text") and not score["text"].get("pass", True):
        names.append("text")
    return names


def cell(score: dict | None) -> str:
    if score is None:
        return "not run"
    worst = score.get("worst_window")
    pct = f"{worst * 100:.1f}%" if isinstance(worst, (int, float)) else "?"
    if score.get("pass"):
        return f"pass ({pct})"
    return f"**FAIL** ({', '.join(failed_checks(score)) or 'score'}; {pct})"


# ---------------------------------------------------------------- sections


def fixtures_section(scores: list[dict], meta: dict[str, dict]) -> tuple[list[str], dict[str, str]]:
    """Returns the lines and a status per fixture (pass, fail, partial, not run)."""
    by: dict[str, dict[str, dict]] = {}
    for s in scores:
        fixture, kind = classify(s)
        by.setdefault(fixture, {})[kind] = s
    lines = ["### Visual fixtures", ""]
    if not scores:
        lines += ["Not run yet: no surimono output has been compared with the Chrome references.", ""]
    lines += [
        "Each island's SVG printed by Chrome (the checkpoint-2 gate), by fulgur, and the whole page printed by surimono + fulgur, "
        "against the Chrome references (`fixtures/README.md`, \"Which comparison gates what\"). Worst-window score in brackets.",
        "",
        "| Fixture | Renderers | Islands, printed by Chrome | Islands, printed by fulgur | Page (surimono + fulgur) |",
        "|---|---|---|---|---|",
    ]
    status: dict[str, str] = {}
    for name in sorted(set(meta) | set(by)):
        got = by.get(name, {})
        n = len(meta.get(name, {}).get("islands", [])) or max([int(k.split("-")[1]) + 1 for k in got if k.startswith("island-")] or [1])
        chrome = [got.get(f"island-{i}") for i in range(n)]
        fulgur = [got.get(f"island-{i}-fulgur") for i in range(n)]
        page = got.get("page")
        everything = chrome + fulgur + [page]
        ran = [s for s in everything if s is not None]
        if not ran:
            status[name] = "not run"
        elif all(s.get("pass") for s in ran):
            status[name] = "pass" if len(ran) == len(everything) else "partial"
        else:
            status[name] = "fail"
        renderers = meta.get(name, {}).get("renderer", "?")
        lines.append(f"| {name} | {renderers} | {'; '.join(cell(s) for s in chrome)} | {'; '.join(cell(s) for s in fulgur)} | {cell(page)} |")
    lines.append("")
    return lines, status


def hostile_section(results_file: Path | None) -> list[str]:
    manifest_file = ROOT / "fixtures" / "hostile" / "manifest.toml"
    lines = ["### Hostile scripts", ""]
    if not manifest_file.exists():
        return lines + ["The hostile suite (`fixtures/hostile/`) is not in this checkout.", ""]
    manifest = tomllib.loads(manifest_file.read_text(encoding="utf-8"))
    max_wall = manifest["limits"]["max_wall_ms"]
    expected: list[tuple[str, int | None, dict]] = []
    for case in manifest["case"]:
        if "steps" in case:
            expected += [(case["id"], i, step) for i, step in enumerate(case["steps"], 1)]
        else:
            expected.append((case["id"], None, case))
    if not results_file:
        return lines + [f"Not run yet: {len(manifest['case'])} cases in `fixtures/hostile/manifest.toml`; the sandbox harness hasn't reported results.", ""]
    results = json.loads(results_file.read_text())
    got = {(r["id"], r.get("step")): r for r in results.get("cases", [])}
    rows = []
    passed = 0
    for cid, step, spec in expected:
        r = got.get((cid, step))
        want = [spec["expect"]] if "expect" in spec else spec["expect_any"]
        name = cid if step is None else f"{cid} step {step}"
        if r is None:
            rows.append((name, " or ".join(want), "not run", "not run"))
            continue
        problems = []
        if r["outcome"] not in want:
            problems.append(f"expected {' or '.join(want)}")
        if r["outcome"] == "Ok" and spec.get("output") is not None and r.get("output") != spec["output"]:
            problems.append(f"output {r.get('output')!r}, expected {spec['output']!r}")
        if "error" in spec and r["outcome"] == "Exception" and not str(r.get("message", "")).startswith(spec["error"]):
            problems.append(f"error {str(r.get('message', ''))[:60]!r}, expected {spec['error']}")
        if isinstance(r.get("wall_ms"), (int, float)) and r["wall_ms"] > max_wall:
            problems.append(f"{r['wall_ms']:.0f} ms, over max_wall_ms {max_wall}")
        if problems:
            rows.append((name, " or ".join(want), f"{r['outcome']} ({r.get('wall_ms', '?')} ms)", "**FAIL**: " + "; ".join(problems)))
        else:
            passed += 1
            rows.append((name, " or ".join(want), f"{r['outcome']} ({r.get('wall_ms', '?')} ms)", "pass"))
    commit = results.get("commit", "unknown")
    lines.append(f"{passed} of {len(expected)} cases end as the manifest expects (results from {commit}).")
    lines.append("")
    failing = [r for r in rows if r[3] != "pass"]
    if failing:
        lines += ["| Case | Expected | Got | Verdict |", "|---|---|---|---|", *[f"| {a} | {b} | {c} | {d} |" for a, b, c, d in failing], ""]
    lines += ["<details><summary>All cases</summary>", "", "| Case | Expected | Got | Verdict |", "|---|---|---|---|"]
    lines += [f"| {a} | {b} | {c} | {d} |" for a, b, c, d in rows]
    lines += ["", "</details>", ""]
    return lines


def real_world_section(fixture_status: dict[str, str], meta: dict[str, dict]) -> list[str]:
    manifest_file = ROOT / "fixtures" / "real-world" / "manifest.toml"
    lines = ["### Real-world libraries", ""]
    if not manifest_file.exists():
        return lines + ["The real-world manifest (`fixtures/real-world/`) is not in this checkout.", ""]
    manifest = tomllib.loads(manifest_file.read_text(encoding="utf-8"))
    renderer_of = {"echarts": "echarts", "vega": "vega", "vega-lite": "vega-lite", "mermaid": "mermaid"}
    lines += ["| Library | Version | Licence | In the plan | Fixtures | Status |", "|---|---|---|---|---|---|"]
    for lib in manifest["library"]:
        r = renderer_of.get(lib["name"], lib["name"])
        fixtures = [name for name, m in meta.items() if r in [x.strip() for x in str(m.get("renderer", "")).split(",")]]
        states = [fixture_status.get(f, "not run") for f in fixtures]
        if not fixtures:
            status = "no fixtures yet"
        elif all(s == "pass" for s in states):
            status = f"pass ({len(fixtures)} of {len(fixtures)} fixtures)"
        elif all(s == "not run" for s in states):
            status = "not run"
        else:
            bad = [f for f, s in zip(fixtures, states) if s == "fail"]
            status = f"**{len(bad)} failing**: {', '.join(bad)}" if bad else "partly run"
        lines.append(f"| {lib['name']} | {lib['version']} | {lib['license']} | {lib['status']} | {len(fixtures)} | {status} |")
    lines.append("")
    return lines


def bench_section(bench_file: Path | None) -> list[str]:
    lines = ["### Speed and memory against Grover", ""]
    if not bench_file:
        return lines + ["Not run yet (`bench/run.sh`).", ""]
    data = json.loads(bench_file.read_text())
    res = data.get("results", {})
    v = data.get("versions", {})
    m = data.get("machine", {})
    lines.append(
        f"From `bench/run.sh` on {'GitHub Actions' if m.get('ci') else 'a local machine'}, commit {v.get('surimono_commit', '?')[:9]}, "
        f"{data.get('method', {}).get('runs', '?')} runs; Chromium {v.get('chromium', '?')}, grover {v.get('grover', '?')}, fulgur {v.get('fulgur_gem', '?')}. "
        "Medians; peak memory is summed PSS over the process tree. The full table is in that run's summary.md."
    )
    lines += ["", "| System | Cold | Warm | Peak memory, cold | Status |", "|---|---|---|---|---|"]
    for key, r in res.items():
        s = r.get("summary", {})
        if r.get("status") != "measured":
            reason = (r.get("reason") or "").splitlines()[0][:140] if r.get("reason") else ""
            lines.append(f"| {r.get('label', key)} | — | — | — | not measured: {reason} |")
            continue
        cold = s.get("cold_ms", {}).get("median")
        warm = s.get("warm_ms", {}).get("median")
        mem = s.get("cold_peak_pss_mb", {}).get("median") if isinstance(s.get("cold_peak_pss_mb"), dict) else None
        note = "lower bound, not surimono" if key == "fulgur-prerendered" else "measured"
        fmt = lambda x, u: f"{x:,.0f} {u}" if isinstance(x, (int, float)) else "—"  # noqa: E731
        lines.append(f"| {r.get('label', key)} | {fmt(cold, 'ms')} | {fmt(warm, 'ms')} | {fmt(mem, 'MB')} | {note} |")
    lines.append("")
    return lines


# ---------------------------------------------------------------- document


def hand_written(existing: str | None) -> str:
    if existing and BEGIN in existing and END in existing:
        return existing.split(BEGIN, 1)[1].split(END, 1)[0].strip("\n") + "\n"
    if existing:
        # The first run: keep whatever follows the generated summary of the old,
        # fully hand-written file (its "## Known gaps" section and everything after).
        i = existing.find("\n## Known gaps")
        if i >= 0:
            return existing[i + 1 :].strip("\n") + "\n"
    return DEFAULT_HAND_WRITTEN


def generate(scores: list[dict], hostile: Path | None, bench: Path | None, commit: str, existing: str | None) -> str:
    meta = fixture_meta()
    fx_lines, fx_status = fixtures_section(scores, meta)
    out = [
        "# Compatibility",
        "",
        "<!-- Generated by fixtures/tools/compat.py; edit only between the hand-written markers. -->",
        "",
        "What surimono does and doesn't do yet, from test results. Every known failure is listed with the test that proves it. "
        "Expectations come from Chrome, specs and WPT, and are never edited to match surimono's output.",
        "",
        f"Results: fixtures {'from ' + commit if scores else 'not run'}; hostile scripts {'from their results file' if hostile else 'not run'}; "
        f"benchmark {'from bench/run.sh' if bench else 'not run'}.",
        "",
        "## Phase 1: islands",
        "",
        *fx_lines,
        *hostile_section(hostile),
        *real_world_section(fx_status, meta),
        *bench_section(bench),
        BEGIN,
        "",
        hand_written(existing).rstrip("\n"),
        "",
        END,
        "",
    ]
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--fixtures", type=Path)
    ap.add_argument("--hostile", type=Path)
    ap.add_argument("--bench", type=Path)
    ap.add_argument("--commit", default="this checkout")
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "COMPAT.md")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    for p in (args.fixtures, args.hostile, args.bench):
        if p and not p.exists():
            sys.exit(f"{p} does not exist")
    existing = args.out.read_text(encoding="utf-8") if args.out.exists() else None
    text = generate(load_scores(args.fixtures), args.hostile, args.bench, args.commit, existing)
    if args.check:
        if text != existing:
            print(f"{args.out} is not what compat.py generates from these results", file=sys.stderr)
            return 1
        print(f"{args.out} is up to date")
        return 0
    args.out.write_text(text, encoding="utf-8")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
