#!/usr/bin/env python3
"""Generates docs/COMPAT.md from machine-readable test results.

    python3 -I fixtures/tools/compat.py [--fixtures DIR] [--hostile FILE] [--bench FILE]
                                        [--commit SHA] [--out docs/COMPAT.md] [--check]

Inputs (each optional; a missing one makes its section say "not run"):

  --fixtures DIR  any directory tree holding score.json files written by
                  fixtures/tools/compare.sh (page mode and island mode, either
                  printer). The fixture is the first word of each file's
                  "subject"; fixtures/islands/*/meta.toml names their renderers.
                  A subject that names no fixture there (another case, a
                  hidden character, a renamed fixture) is an error.
  --hostile FILE  the hostile-suite results (format below). Pass or fail is
                  decided here, against fixtures/hostile/manifest.toml, not
                  taken from the file.
  --bench FILE    bench/run.sh's results.json.

The real-world section comes from fixtures/real-world/manifest.toml plus the
fixture results of each library's renderer.

Who runs it: the conformance researcher, after merges that change results,
from one CI run's published results (git show origin/ci-results:runs/<sha>/...),
with --commit <that sha>, and commits docs/COMPAT.md in a PR.

Hand-written sections survive regeneration: everything between the lines
<!-- hand-written: begin --> and <!-- hand-written: end --> in the existing
--out file is copied into the new one unchanged. --check exits 1 when the
generated file differs from --out (nothing is written).

A result can never hide another: two score.json files for the same fixture and
comparison, or two hostile results for the same case and step, are an error
that names both. A hostile result whose case or step isn't in the manifest is
listed as failing and counted. A score.json with "error" (the render failed
before anything could be compared) is a failure, never "not run". A
determinism.json (checkpoint.mjs: two renders in two processes) that says the
renders differ fails its fixture. Results stamped by the stand-in CLI
(fake-surimono.mjs) or by two different CLIs are an error.

The hostile results format is fixtures/hostile/RESULTS_SCHEMA.md (schema 1),
one entry per case and step:

    {"schema": 1, "commit": "<sha>",
     "cases": [{"id": "infinite-loop", "step": null, "outcome": "TimeLimit",
                "output": null, "message": "...", "wall_ms": 2001}, ...]}
"""

from __future__ import annotations

import argparse
import decimal
import difflib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class CompatError(Exception):
    """Results that can't be reported honestly (duplicates, bad files)."""
BEGIN = "<!-- hand-written: begin -->"
END = "<!-- hand-written: end -->"

DEFAULT_HAND_WRITTEN = """## Known gaps

None recorded yet.
"""


# ---------------------------------------------------------------- inputs


def _reject_constant(name: str):
    raise ValueError(f"{name} is not valid JSON")


def read_json(path: Path):
    """A results file as JSON.parse reads it: Python's extensions NaN,
    Infinity and -Infinity are refused (no producer writes them, and
    lib/report.mjs couldn't read the file), so they can't reach a verdict."""
    try:
        return json.loads(path.read_text(), parse_constant=_reject_constant)
    except (OSError, ValueError) as e:
        raise CompatError(f"{path}: {e}")


def load_scores(directory: Path | None) -> list[dict]:
    if not directory:
        return []
    scores = []
    for f in sorted(directory.rglob("score.json")):
        data = read_json(f)
        if not isinstance(data, dict):
            raise CompatError(f"{f}: not a JSON object")
        data["_file"] = str(f)
        scores.append(data)
    return scores


def load_determinism(directory: Path | None) -> dict[str, dict]:
    """checkpoint.mjs's determinism.json files, by fixture; a duplicate is an error."""
    if not directory:
        return {}
    out: dict[str, dict] = {}
    for f in sorted(directory.rglob("determinism.json")):
        data = read_json(f)
        if not isinstance(data, dict):
            raise CompatError(f"{f}: not a JSON object")
        data["_file"] = str(f)
        name = data.get("subject")
        if not isinstance(name, str) or not name:
            raise CompatError(f"{f}: no \"subject\" naming the fixture")
        if name in out:
            raise CompatError(f"two determinism results for {name}: {out[name]['_file']} and {f}")
        out[name] = data
    return out


NOT_RECORDED = {"cli": None, "version": "producer not recorded", "sha256": None, "commit": None}


def check_producers(records: list[dict]) -> dict | None:
    """The one surimono CLI that produced these results. A stand-in
    (fixtures/tools/test/fake-surimono.mjs prints Chrome's own SVGs) is an
    error, and so is a mix of CLIs. A result without a stamp (compare.mjs run
    by hand, on any SVG) counts as one more, unknown CLI, so it can't be
    credited to a stamped one; an absent, null or empty producer is that
    unknown CLI, and any other value that isn't an object is an error.
    Returns NOT_RECORDED when nothing is stamped, None when there are no
    results."""
    seen: dict[tuple, dict] = {}
    for r in records:
        p = r.get("producer")
        if p is None or p == {}:
            p = NOT_RECORDED
        if not isinstance(p, dict):
            raise CompatError(f"{r.get('_file', '?')}: producer {p!r} is not an object with cli, version, sha256 and commit")
        if "fake-surimono" in str(p.get("version", "")) or "fake-surimono" in str(p.get("cli", "")):
            raise CompatError(f"{r.get('_file', '?')}: produced by the stand-in {p.get('cli')} ({p.get('version')}), not by surimono")
        # Compared as JSON text, so any value (a list, an object) can be a key.
        key = tuple(json.dumps(p.get(k), sort_keys=True) for k in ("cli", "version", "sha256", "commit"))
        seen.setdefault(key, {**p, "_file": r.get("_file", "?")})
    if len(seen) > 1:
        a, b = list(seen.values())[:2]
        raise CompatError(
            f"results from two different surimono CLIs ({a['_file']}: {a.get('version')}; {b['_file']}: {b.get('version')}); "
            "a result without a producer stamp counts as its own; pass one run's directory"
        )
    return next(iter(seen.values()), None)


def classify(score: dict) -> tuple[str, str]:
    """(fixture, comparison) where comparison is page, island-N or island-N-fulgur.

    Only compare.mjs's own subjects are accepted: "<fixture>" for a page and
    "<fixture> island N (printed by chrome|fulgur)" for an island. Anything
    else, such as baseline.mjs's "<fixture> island N" or "<fixture> page"
    (fulgur printing Chrome's SVGs, not surimono's), is an error rather than a
    result in surimono's columns."""
    subject = score.get("subject", "")
    where = score.get("_file", "?")
    # ASCII digits, no leading zero: one island has exactly one spelling.
    m = re.fullmatch(r"(\S+) island (0|[1-9][0-9]*) \(printed by (chrome|fulgur)\)", subject)
    if m:
        return m.group(1), f"island-{m.group(2)}" + ("-fulgur" if m.group(3) == "fulgur" else "")
    if re.fullmatch(r"\S+", subject):
        return subject, "page"
    raise CompatError(
        f"{where}: subject {subject!r} is not a surimono comparison from compare.mjs or checkpoint.mjs "
        "(a baseline.mjs score is fulgur printing Chrome's SVGs); pass a directory with surimono's results only"
    )


def fixture_meta() -> dict[str, dict]:
    meta = {}
    for f in sorted((ROOT / "fixtures" / "islands").glob("*/meta.toml")):
        meta[f.parent.name] = tomllib.loads(f.read_text(encoding="utf-8"))
    return meta


def _set(value) -> bool:
    """Whether a JSON value is set, as JavaScript reads it (and lib/report.mjs
    with it): anything but absent, null, false, 0 and "". An empty object or
    list is set, so `"error": {}` is an error. (NaN, which JSON can't hold and
    read_json refuses, would count as set.)"""
    if value is None or value is False or value == "":
        return False
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return value != 0
    return True


def _js_string(value) -> str:
    """A JSON value as JavaScript's String() spells it, so a check named in a
    page's "failed" list reads the same here as in lib/report.mjs: null,
    true, false, 1 (not 1.0), 1e-7, "a,b" for a list, [object Object]."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return value
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        if value != value:
            return "NaN"
        if value in (float("inf"), float("-inf")):
            return "Infinity" if value > 0 else "-Infinity"
        if value == 0:
            return "0"
        # repr has the shortest round-trip digits, as JavaScript does; only the
        # layout differs: JavaScript is positional for exponents -7 < e < 21.
        exact = decimal.Decimal(repr(value))
        mantissa, exponent = f"{exact:e}".split("e")
        if -7 < int(exponent) < 21:
            text = format(exact, "f")
            return text[:-2] if text.endswith(".0") else text
        if "." in mantissa:
            mantissa = mantissa.rstrip("0").rstrip(".")
        return f"{mantissa}e{'+' if int(exponent) > 0 else '-'}{abs(int(exponent))}"
    if isinstance(value, list):
        return ",".join("" if x is None else _js_string(x) for x in value)
    return "[object Object]"


def _check_failed(check) -> bool:
    """A check that is present failed unless it is an object whose `pass` is
    exactly true: a missing `pass`, the string "false" or anything else fails."""
    return not (isinstance(check, dict) and check.get("pass") is True)


def failed_checks(score: dict) -> list[str]:
    """The checks a score failed, by name (from compare.mjs's per-page "failed"
    lists). A score that compared no page at all fails with "no pages"."""
    names: list[str] = []

    def add(name: str) -> None:
        if name not in names:
            names.append(name)

    if _set(score.get("error")):
        add("render error")
    pages = score.get("pages")
    if not isinstance(pages, list):
        pages = []
    if not pages and not _set(score.get("error")):
        add("no pages")
    for p in pages:
        listed = p.get("failed") if isinstance(p, dict) else None
        if isinstance(listed, list):
            for n in listed:
                add(_js_string(n))
            if not listed and _check_failed(p):
                add("page")
        elif _set(listed) or _check_failed(p):
            # A "failed" that isn't a list (an object, a string) fails the page.
            add("page")
    for check in ("text", "font"):
        if score.get(check) is not None and _check_failed(score[check]):
            add(check)
    return names


def _pct(x) -> str:
    return f"{x * 100:.1f}%" if isinstance(x, (int, float)) else "?"


def check_detail(score: dict, check: str) -> str:
    """One failed check with its own number: shape, colour and hue with their
    worst windows, text with what is missing or extra, size with both sizes."""
    pages = [p for p in (score.get("pages") if isinstance(score.get("pages"), list) else []) if isinstance(p, dict)]
    if check == "render error":
        return f"render error: {str(score.get('error')).splitlines()[0][:120]}"
    if check == "shape":
        return f"shape {_pct(score.get('worst_window'))}"
    if check in ("colour", "hue"):
        top = score.get(f"worst_{check}_window")
        per_page = [p[f"{check}_worst_window"] for p in pages if isinstance(p.get(f"{check}_worst_window"), (int, float))]
        return f"{check} {_pct(top if top is not None else (max(per_page) if per_page else None))}"
    if check == "no pages":
        return "no pages compared"
    if check == "text":
        t = score.get("text") if isinstance(score.get("text"), dict) else {}
        parts = [f"{len(t[k])} {k}" for k in ("missing", "extra") if isinstance(t.get(k), list) and t[k]]
        return f"text: {', '.join(parts) or 'differs'}"
    if check == "font":
        f = score.get("font") if isinstance(score.get("font"), dict) else {}
        lists = {k: f[k] if isinstance(f.get(k), list) else [] for k in ("mismatches", "missing", "extra")}
        mm = [m for m in lists["mismatches"] if isinstance(m, dict)]
        parts = [f"{len(lists['mismatches'])} differ"] if lists["mismatches"] else []
        parts += [f"{len(lists[k])} {k}" for k in ("missing", "extra") if lists[k]]
        first = f" ({mm[0].get('key')}: Chrome {mm[0].get('chrome')}, surimono {mm[0].get('surimono')})" if mm else ""
        return f"font: {', '.join(parts) or 'differs'}{first}"
    if check == "size":
        sizes = [f"{p.get('actual_size')} vs Chrome {p.get('expected_size')}" for p in pages if p.get("size_match") is False]
        return f"size {sizes[0]}" if sizes else "size"
    if check == "page":
        missing = [p.get("detail") for p in pages if p.get("worst_window") is None and p.get("detail")]
        return f"page: {missing[0]}" if missing else f"page (shape {_pct(score.get('worst_window'))})"
    return check


def passed(score: dict) -> bool:
    """A pass is derived, never taken on trust: `pass` must be the boolean true,
    at least one page must have been compared and no check inside the score may
    have failed, so a producer that forgets to fold a new check into `pass`
    can't turn it into a pass."""
    return score.get("pass") is True and not _set(score.get("error")) and not failed_checks(score)


def cell(score: dict | None) -> str:
    if score is None:
        return "not run"
    if passed(score):
        return f"pass (shape {_pct(score.get('worst_window'))})"
    checks = failed_checks(score) or ["shape"]
    return f"**FAIL** ({'; '.join(check_detail(score, c) for c in checks)})"


# ---------------------------------------------------------------- sections


def unknown_fixture(record: dict, what: str, name: str, meta: dict[str, dict]) -> CompatError:
    """The error for a result whose fixture isn't in fixtures/islands/: its own
    row would show renderer "?", and the real-world table, which groups by
    meta.toml's renderer, would never count it."""
    near = [n for n in meta if n.casefold() == name.casefold()] or difflib.get_close_matches(name, list(meta), n=1)
    return CompatError(
        f"{record.get('_file', '?')}: {what} for {name!r}, which is not a fixture in fixtures/islands/ (no meta.toml)"
        + (f"; did you mean {near[0]!r}?" if near else "")
        + "; pass results for this checkout's fixtures only"
    )


def fixtures_section(scores: list[dict], meta: dict[str, dict], determinism: dict[str, dict] | None = None) -> tuple[list[str], dict[str, str]]:
    """Returns the lines and a status per fixture (pass, fail, partial, not run)."""
    by: dict[str, dict[str, dict]] = {}
    for s in scores:
        fixture, kind = classify(s)
        if fixture not in meta:
            raise unknown_fixture(s, "a result", fixture, meta)
        if kind in by.get(fixture, {}):
            first = by[fixture][kind].get("_file", "?")
            raise CompatError(
                f"two results for {fixture} {kind}: {first} and {s.get('_file', '?')}; "
                "pass one run's directory, so one result can't hide the other"
            )
        by.setdefault(fixture, {})[kind] = s
    lines = ["### Visual fixtures", ""]
    if not scores:
        lines += ["Not run yet: no surimono output has been compared with the Chrome references.", ""]
    lines += [
        "Each island's SVG printed by Chrome (the checkpoint-2 gate), by fulgur, and the whole page printed by surimono + fulgur, "
        "against the Chrome references (`fixtures/README.md`, \"Which comparison gates what\"). A pass shows the shape score (worst window); "
        "a failure names each failed check with its own number (shape, colour and hue windows, text strings missing or extra, font weight or style, size).",
        "",
        "| Fixture | Renderers | Islands, printed by Chrome | Islands, printed by fulgur | Page (surimono + fulgur) | Byte-identical (two processes) |",
        "|---|---|---|---|---|---|",
    ]
    for name, det in (determinism or {}).items():
        if name not in meta:
            raise unknown_fixture(det, "a determinism result", name, meta)
    status: dict[str, str] = {}
    for name in sorted(set(meta) | set(by) | set(determinism or {})):
        got = by.get(name, {})
        n = len(meta.get(name, {}).get("islands", [])) or max([int(k.split("-")[1]) + 1 for k in got if k.startswith("island-")] or [1])
        # Every result must land in a column the table shows; anything else
        # (an island the fixture doesn't have, any other key) stops the run.
        shown = {"page"} | {f"island-{i}" for i in range(n)} | {f"island-{i}-fulgur" for i in range(n)}
        stray = [k for k in got if k not in shown]
        if stray:
            raise CompatError(
                f"{got[stray[0]].get('_file', '?')}: a result for {name} {stray[0]}, which the table has no place for "
                f"(the fixture has {n} island{'' if n == 1 else 's'}, meta.toml); one result can't hide another"
            )
        chrome = [got.get(f"island-{i}") for i in range(n)]
        fulgur = [got.get(f"island-{i}-fulgur") for i in range(n)]
        page = got.get("page")
        everything = chrome + fulgur + [page]
        ran = [s for s in everything if s is not None]
        det = (determinism or {}).get(name)
        if det is None:
            det_cell = "not run"
        elif det.get("deterministic") is True:
            det_cell = "yes"
        else:
            det_cell = f"**FAIL** ({det.get('note') or 'the two renders differ'})"
        if not ran:
            status[name] = "not run"
        elif all(passed(s) for s in ran) and (det is None or det.get("deterministic") is True):
            status[name] = "pass" if len(ran) == len(everything) and det is not None else "partial"
        else:
            status[name] = "fail"
        renderers = meta.get(name, {}).get("renderer", "?")
        lines.append(f"| {name} | {renderers} | {'; '.join(cell(s) for s in chrome)} | {'; '.join(cell(s) for s in fulgur)} | {cell(page)} | {det_cell} |")
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
    results = read_json(results_file)
    got: dict[tuple[str, int | None], dict] = {}
    for r in results.get("cases", []):
        key = (r.get("id"), r.get("step"))
        if key in got:
            raise CompatError(
                f"{results_file}: two results for hostile case {key[0]!r} step {key[1]!r} "
                f"({got[key].get('outcome')} and {r.get('outcome')}); one result can't hide another"
            )
        got[key] = r
    known = {(cid, step) for cid, step, _ in expected}
    unknown = [r for key, r in got.items() if key not in known]
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
        if not r.get("outcome"):
            problems.append("no outcome in the results file")
        elif r["outcome"] not in want:
            problems.append(f"expected {' or '.join(want)}")
        if r.get("outcome") == "Ok" and spec.get("output") is not None and r.get("output") != spec["output"]:
            problems.append(f"output {r.get('output')!r}, expected {spec['output']!r}")
        if "error" in spec and r.get("outcome") == "Exception" and not str(r.get("message", "")).startswith(spec["error"]):
            problems.append(f"error {str(r.get('message', ''))[:60]!r}, expected {spec['error']}")
        wall = r.get("wall_ms")
        if not isinstance(wall, (int, float)) or isinstance(wall, bool):
            # Required by RESULTS_SCHEMA.md: without it the hang check can't be made.
            problems.append("no wall_ms, so the max_wall_ms check can't be made")
        elif wall > max_wall:
            problems.append(f"{wall:.0f} ms, over max_wall_ms {max_wall}")
        if problems:
            rows.append((name, " or ".join(want), f"{r.get('outcome') or '—'} ({r.get('wall_ms', '?')} ms)", "**FAIL**: " + "; ".join(problems)))
        else:
            passed += 1
            rows.append((name, " or ".join(want), f"{r['outcome']} ({r.get('wall_ms', '?')} ms)", "pass"))
    for r in unknown:
        name = r.get("id") if r.get("step") is None else f"{r.get('id')} step {r.get('step')}"
        rows.append((name, "—", f"{r.get('outcome')} ({r.get('wall_ms', '?')} ms)", "**FAIL**: not in the manifest"))
    commit = results.get("commit", "unknown")
    total = len(expected) + len(unknown)
    extra = f"; {len(unknown)} not in the manifest" if unknown else ""
    lines.append(f"{passed} of {total} cases end as the manifest expects (results from {commit}{extra}).")
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
    data = read_json(bench_file)
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
        note = "reference point, not surimono" if key == "fulgur-prerendered" else "measured"
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


def generate(scores: list[dict], hostile: Path | None, bench: Path | None, commit: str, existing: str | None, determinism: dict[str, dict] | None = None) -> str:
    meta = fixture_meta()
    producer = check_producers(scores + list((determinism or {}).values()))
    fx_lines, fx_status = fixtures_section(scores, meta, determinism)
    made_by = ""
    if producer and producer.get("cli") is None:
        made_by = " (producer not recorded)"
    elif producer:
        made_by = f" (surimono CLI {producer.get('version')}, sha256 {str(producer.get('sha256'))[:12]}{', commit ' + producer['commit'] if producer.get('commit') else ''})"
    out = [
        "# Compatibility",
        "",
        "<!-- Generated by fixtures/tools/compat.py; edit only between the hand-written markers. -->",
        "",
        "What surimono does and doesn't do yet, from test results. Every known failure is listed with the test that proves it. "
        "Expectations come from Chrome, specs and WPT, and are never edited to match surimono's output.",
        "",
        f"Results: fixtures {'from ' + commit + made_by if scores else 'not run'}; hostile scripts {'from their results file' if hostile else 'not run'}; "
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
    try:
        text = generate(load_scores(args.fixtures), args.hostile, args.bench, args.commit, existing, load_determinism(args.fixtures))
    except CompatError as e:
        print(f"compat.py: {e}", file=sys.stderr)
        return 1
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
