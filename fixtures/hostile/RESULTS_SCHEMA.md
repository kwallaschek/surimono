# Hostile-suite results file (schema 1)

The runtime engineer's harness (P1-RT-11) runs every case in
`manifest.toml` and writes one JSON file in this format.
`fixtures/tools/compat.py --hostile <file>` turns it into the "Hostile
scripts" section of `docs/COMPAT.md`. The harness reports what happened; it
never decides pass or fail. `compat.py` decides that against the manifest,
so a harness bug can't turn a failure into a pass.

```json
{
  "schema": 1,
  "commit": "4b2a7bec6d0bfa769f55a45591968792fe4a3979",
  "harness": "crates/core/tests/hostile.rs",
  "cases": [
    { "id": "infinite-loop", "step": null, "outcome": "TimeLimit",
      "output": null, "message": "interrupted", "wall_ms": 2003.4 },
    { "id": "pool-isolation", "step": 1, "outcome": "Ok",
      "output": "set", "message": null, "wall_ms": 1.2 },
    { "id": "pool-isolation", "step": 2, "outcome": "Ok",
      "output": "clean", "message": null, "wall_ms": 0.9 }
  ]
}
```

## Fields

| Field | Type | Required | Meaning |
|---|---|---|---|
| `schema` | integer | yes | `1` |
| `commit` | string | yes | Full SHA of the checkout the harness ran on (`git rev-parse HEAD`) |
| `harness` | string | no | What produced the file, e.g. the test's path |
| `cases` | array | yes | One entry per case, and per step for `steps` cases |
| `cases[].id` | string | yes | The case's `id` in `manifest.toml`, exactly |
| `cases[].step` | integer or null | yes | `1`, `2`, ... for the steps of a `steps` case, in manifest order; `null` for a single-file case |
| `cases[].outcome` | string | yes | One of the outcomes below |
| `cases[].output` | string or null | yes | The result string when `outcome` is `Ok`, byte for byte; `null` otherwise |
| `cases[].message` | string or null | yes | The error's message for `Exception` (it starts with the JS error type, e.g. `"ReferenceError: require is not defined"`); any detail for the other outcomes; `null` if none |
| `cases[].wall_ms` | number | yes | Wall-clock milliseconds from the start of the case (or step) to its outcome, including draining the job queue |

## Outcomes

These match the manifest's header, plus two that only the harness can see:

| `outcome` | When |
|---|---|
| `MemoryLimit` | `SandboxError::MemoryLimit` |
| `TimeLimit` | `SandboxError::TimeLimit` |
| `StackOverflow` | `SandboxError::StackOverflow` |
| `Exception` | `SandboxError::Exception` or `SandboxError::Type` (a thrown error, an unusable result, or a promise that can't settle) |
| `Ok` | The script finished with a string result (`output`) |
| `Crash` | The process running the case died (signal, abort). Run each case in a child process, or report the whole run as crashed, so one crash can't hide the cases after it |
| `Hang` | The case didn't end within the manifest's `max_wall_ms`. Kill it and report `Hang` with `wall_ms` at the kill |

## Rules `compat.py` enforces

- **One entry per (`id`, `step`).** A duplicate is an error: `compat.py` exits 1 and names both entries. Don't merge two runs into one file.
- **Every entry must be in the manifest.** An entry whose `id` or `step` isn't in `manifest.toml` is listed as **FAIL: not in the manifest** and counts against the total. It is never dropped.
- **Missing entries read "not run"**, which is not a pass.
- **Verdicts come from the manifest**, never from the file. Any `pass` field in the file is ignored. A case passes when:
  - `outcome` is `expect`, or one of `expect_any`;
  - with `Ok` and a manifest `output`, `output` equals it exactly;
  - with `Exception` and a manifest `error`, `message` starts with it;
  - `wall_ms` is at most `max_wall_ms`.
