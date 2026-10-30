//! The hostile-script suite, `fixtures/hostile/manifest.toml`, through the
//! island runner: every case must end in its expected typed outcome, within
//! `max_wall_ms`, without crashing or hanging.
//!
//! Each case runs in a child process (this test binary again, running
//! `hostile_child`) under a watchdog, so a hang or a crash fails that case
//! instead of the whole run. A case's script is evaluated as a classic
//! global script inside an island render: the bundle's entry hands the file
//! to an indirect `eval` and returns its completion value, which the runner
//! settles (a Promise is awaited while microtasks drain) and requires to be a
//! string. `steps` cases render their files one after another on a
//! one-worker pool, so the pool's reset must isolate them.
//!
//! Set `SURIMONO_HOSTILE_RESULTS=<file>` to write the results in schema 1
//! (see `fixtures/tools/compat.py`), for docs/COMPAT.md.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use surimono_islands::SandboxError;
use surimono_islands::{IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions};

/// Cases that fail today, with the reason. Each one is also in docs/COMPAT.md;
/// the expectation itself is never changed.
const KNOWN_FAILURES: &[(&str, &str)] = &[];

/// Time a child may take beyond the cases' own `max_wall_ms`: starting the
/// process and the runner, and a debug build on a busy machine.
const CHILD_SLACK: Duration = Duration::from_secs(20);

/// Children running at once.
const PARALLEL: usize = 2;

/// Marks the child's result lines on stdout.
const RESULT_PREFIX: &str = "HOSTILE-RESULT ";

#[derive(Debug, Deserialize)]
struct Manifest {
    limits: ManifestLimits,
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct ManifestLimits {
    memory_bytes: usize,
    time_ms: u64,
    stack_bytes: usize,
    max_wall_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct Expect {
    expect: Option<String>,
    expect_any: Option<Vec<String>>,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    file: Option<String>,
    #[serde(flatten)]
    expect: Expect,
    steps: Option<Vec<Step>>,
}

#[derive(Debug, Deserialize)]
struct Step {
    file: String,
    #[serde(flatten)]
    expect: Expect,
}

impl Case {
    /// The files to render, with what each must give.
    fn steps(&self) -> Vec<(String, Expect)> {
        match (&self.steps, &self.file) {
            (Some(steps), _) => steps
                .iter()
                .map(|s| (s.file.clone(), s.expect.clone()))
                .collect(),
            (None, Some(file)) => vec![(file.clone(), self.expect.clone())],
            (None, None) => panic!("case {} has neither file nor steps", self.id),
        }
    }
}

/// What one render (one step) did, as schema 1 records it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
struct Outcome {
    outcome: String,
    output: Option<String>,
    message: Option<String>,
    wall_ms: u64,
}

fn hostile_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hostile")
}

fn manifest() -> Manifest {
    let path = hostile_dir().join("manifest.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A bundle whose entry evaluates `source` as a classic global script and
/// returns its completion value.
fn wrapper(source: &str) -> String {
    // A JSON string is a valid JavaScript string literal.
    let literal = serde_json::to_string(source).unwrap();
    format!(
        "var __hostileSource = {literal};\nfunction entry() {{ return (0, eval)(__hostileSource); }}\n"
    )
}

fn outcome_of(result: Result<String, IslandError>, wall: Duration) -> Outcome {
    let wall_ms = u64::try_from(wall.as_millis()).unwrap_or(u64::MAX);
    let (outcome, output, message) = match result {
        Ok(s) => ("Ok", Some(s), None),
        Err(IslandError::Sandbox(SandboxError::MemoryLimit)) => ("MemoryLimit", None, None),
        Err(IslandError::Sandbox(SandboxError::TimeLimit)) => ("TimeLimit", None, None),
        Err(IslandError::Sandbox(SandboxError::StackOverflow)) => ("StackOverflow", None, None),
        Err(IslandError::Sandbox(SandboxError::Exception { message, .. })) => {
            ("Exception", None, Some(message))
        }
        Err(e @ IslandError::Sandbox(SandboxError::Type { .. })) => {
            ("Exception", None, Some(e.to_string()))
        }
        Err(e @ IslandError::NotAString { .. }) => ("Exception", None, Some(e.to_string())),
        // Anything else is the engine or the runner failing, not the script.
        Err(e) => ("Crash", None, Some(e.to_string())),
    };
    Outcome {
        outcome: outcome.to_owned(),
        output,
        message,
        wall_ms,
    }
}

/// Child side: runs the case named in SURIMONO_HOSTILE_CASE and prints one
/// result line per step.
#[test]
#[ignore = "helper: run in a child process by hostile_suite"]
fn hostile_child() {
    let id = std::env::var("SURIMONO_HOSTILE_CASE").unwrap();
    let m = manifest();
    let case = m.case.iter().find(|c| c.id == id).unwrap();
    let limits = Limits::new(
        m.limits.memory_bytes,
        Duration::from_millis(m.limits.time_ms),
        m.limits.stack_bytes,
    );
    // One worker, so `steps` render one after another in the same pool.
    let runner = IslandRunner::with_estimated_text_metrics(RunnerOptions {
        pool_size: 1,
        limits,
        ..RunnerOptions::default()
    })
    .unwrap();
    for (i, (file, _)) in case.steps().iter().enumerate() {
        let source = std::fs::read_to_string(hostile_dir().join(file)).unwrap();
        let name = format!("step-{i}");
        runner
            .register_bundle(&name, &wrapper(&source), "entry")
            .unwrap();
        let t = Instant::now();
        let result = runner.render(&IslandRequest {
            renderer: &name,
            spec_json: "{}",
            width: 100,
            height: 100,
        });
        let outcome = outcome_of(result, t.elapsed());
        println!(
            "\n{RESULT_PREFIX}{}",
            serde_json::to_string(&outcome).unwrap()
        );
    }
}

/// Runs one case in a child process under a watchdog.
fn run_case(exe: &Path, case: &Case, max_wall: Duration) -> Vec<Outcome> {
    let steps = case.steps().len();
    let started = Instant::now();
    let mut child = Command::new(exe)
        .args([
            "hostile_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SURIMONO_HOSTILE_CASE", &case.id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = max_wall * u32::try_from(steps).unwrap() + CHILD_SLACK;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let mut outcomes: Vec<Outcome> = stdout
        .lines()
        // libtest may print "test hostile_child ... " on the same line.
        .filter_map(|l| l.find(RESULT_PREFIX).map(|i| &l[i + RESULT_PREFIX.len()..]))
        .map(|json| serde_json::from_str(json).unwrap())
        .collect();
    // Steps the child never reported hung or crashed it.
    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    while outcomes.len() < steps {
        let (outcome, message) = match status {
            None => ("Hang", "killed by the watchdog".to_owned()),
            Some(s) => (
                "Crash",
                format!(
                    "child exited with {s}: {}",
                    stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
                ),
            ),
        };
        outcomes.push(Outcome {
            outcome: outcome.to_owned(),
            output: None,
            message: Some(message),
            wall_ms,
        });
    }
    // Past max_wall_ms is a hang, whatever the outcome.
    for o in &mut outcomes {
        if o.wall_ms > u64::try_from(max_wall.as_millis()).unwrap() && o.outcome != "Hang" {
            o.message = Some(format!(
                "{} after {} ms: {:?}",
                o.outcome, o.wall_ms, o.message
            ));
            o.outcome = "Hang".to_owned();
        }
    }
    outcomes
}

/// Whether `o` meets `want`; on failure, why not.
fn check(o: &Outcome, want: &Expect) -> Result<(), String> {
    let allowed: Vec<&str> = match (&want.expect, &want.expect_any) {
        (Some(e), _) => vec![e.as_str()],
        (None, Some(any)) => any.iter().map(String::as_str).collect(),
        (None, None) => return Err("the manifest gives no expectation".into()),
    };
    if !allowed.contains(&o.outcome.as_str()) {
        return Err(format!(
            "got {} ({:?}), want {}",
            o.outcome,
            o.message.as_deref().or(o.output.as_deref()).unwrap_or(""),
            allowed.join(" or ")
        ));
    }
    if o.outcome == "Ok" {
        if let Some(expected) = &want.output {
            if o.output.as_deref() != Some(expected.as_str()) {
                return Err(format!("output {:?}, want {expected:?}", o.output));
            }
        }
    }
    if o.outcome == "Exception" {
        if let Some(error) = &want.error {
            let message = o.message.as_deref().unwrap_or("");
            if !message.starts_with(error.as_str()) {
                return Err(format!("exception {message:?}, want a {error}"));
            }
        }
    }
    Ok(())
}

#[test]
fn hostile_suite() {
    let m = manifest();
    let max_wall = Duration::from_millis(m.limits.max_wall_ms);
    let exe = std::env::current_exe().unwrap();
    let results: Mutex<BTreeMap<usize, Vec<Outcome>>> = Mutex::new(BTreeMap::new());
    let next = Mutex::new(0usize);
    std::thread::scope(|s| {
        for _ in 0..PARALLEL {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(case) = m.case.get(i) else { break };
                    let outcomes = run_case(&exe, case, max_wall);
                    results.lock().unwrap().insert(i, outcomes);
                }
            });
        }
    });
    let results = results.into_inner().unwrap();

    let mut entries = Vec::new();
    let mut failures = Vec::new();
    let mut known = Vec::new();
    for (i, case) in m.case.iter().enumerate() {
        let steps = case.steps();
        let multi = steps.len() > 1 || case.steps.is_some();
        for (n, ((_, want), o)) in steps.iter().zip(&results[&i]).enumerate() {
            let step = multi.then_some(n + 1);
            let verdict = check(o, want);
            let label = match step {
                Some(n) => format!("{} step {n}", case.id),
                None => case.id.clone(),
            };
            eprintln!(
                "{:<5} {label:<36} {:<13} {:>6} ms  {}",
                if verdict.is_ok() { "pass" } else { "FAIL" },
                o.outcome,
                o.wall_ms,
                verdict.as_ref().err().map_or("", String::as_str)
            );
            if let Err(why) = verdict {
                match KNOWN_FAILURES.iter().find(|(id, _)| *id == case.id) {
                    Some((_, reason)) => known.push(format!("{label}: {why} (known: {reason})")),
                    None => failures.push(format!("{label}: {why}")),
                }
            }
            entries.push(serde_json::json!({
                "id": case.id,
                "step": step,
                "outcome": o.outcome,
                "output": o.output,
                "message": o.message,
                "wall_ms": o.wall_ms,
            }));
        }
    }
    if let Some(path) = std::env::var_os("SURIMONO_HOSTILE_RESULTS") {
        // CI's commit, or this checkout's.
        let commit = std::env::var("GITHUB_SHA").ok().or_else(|| {
            let out = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        });
        let doc = serde_json::json!({"schema": 1, "commit": commit, "cases": entries});
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        eprintln!(
            "hostile: results written to {}",
            PathBuf::from(path).display()
        );
    }
    eprintln!(
        "hostile: {} cases, {} step results, {} unexpected failures, {} known failures",
        m.case.len(),
        entries.len(),
        failures.len(),
        known.len()
    );
    for k in &known {
        eprintln!("hostile: known failure: {k}");
    }
    assert!(
        failures.is_empty(),
        "unexpected failures:\n{}",
        failures.join("\n")
    );
}
