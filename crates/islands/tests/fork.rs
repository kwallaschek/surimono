//! Forking a process that renders islands (P1-RT-30). Puma, Unicorn and
//! Resque fork after loading the app, possibly while another thread is
//! rendering. A forked child must never hang: a runner from the parent fails
//! at once there (its workers didn't survive the fork, and a thread that
//! didn't may have held one of its locks), and a new runner in the child
//! renders as usual, built-ins included.
//!
//! The forking needs `unsafe` (libc's `fork`, `waitpid`, `_exit`); this
//! test module is opted in, and every block has a `SAFETY:` comment.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use surimono_islands::{IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions};

/// A small registered renderer that does some work, so a render holds the
/// runner's locks and runs the sandbox for a while.
const TINY: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  let s = 0;
  for (let i = 0; i < spec.n; i++) s += i % 7;
  return '<svg xmlns="http://www.w3.org/2000/svg" width="' + ctx.width + '"><desc>' + s + '</desc></svg>';
}
"#;

fn runner() -> IslandRunner {
    let r = IslandRunner::with_estimated_text_metrics(RunnerOptions {
        pool_size: 2,
        limits: Limits::default().with_time(Duration::from_secs(5)),
        ..RunnerOptions::default()
    })
    .unwrap();
    r.register_bundle("tiny", TINY, "entry").unwrap();
    r
}

fn tiny(r: &IslandRunner, n: u32) -> Result<String, IslandError> {
    r.render(&IslandRequest {
        renderer: "tiny",
        spec_json: &format!("{{\"n\": {n}}}"),
        width: 10,
        height: 10,
    })
}

#[cfg(feature = "echarts")]
fn bar_chart(r: &IslandRunner) -> Result<String, IslandError> {
    let spec = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/echarts/bar.json"
    ))
    .map_err(|e| IslandError::Worker {
        message: e.to_string(),
    })?;
    r.render(&IslandRequest {
        renderer: "echarts",
        spec_json: &spec,
        width: 300,
        height: 200,
    })
}

/// What a forked child checks; 0 when all is well.
fn child(shared: &IslandRunner, owned: Option<IslandRunner>, i: usize) -> i32 {
    // The parent's runners: every call returns at once, without their locks.
    match tiny(shared, 10) {
        Err(IslandError::Worker { message }) if message.contains("another process") => {}
        _ => return 10,
    }
    if shared.register_bundle("other", TINY, "entry").is_ok() {
        return 11;
    }
    let stats = shared.stats();
    if stats.workers != 0 || stats.idle_workers != 0 || stats.runtimes != 0 {
        return 12;
    }
    if !format!("{shared:?}").contains("forked") || !shared.renderers().is_empty() {
        return 13;
    }
    // Dropping one leaks it instead of joining threads that don't exist.
    drop(owned);
    // A new runner renders, with a registered bundle and a built-in.
    let fresh = runner();
    match tiny(&fresh, 1000) {
        Ok(svg) if svg.contains("<desc>2997</desc>") => {}
        _ => return 20,
    }
    #[cfg(feature = "echarts")]
    if i % 10 == 0 {
        match bar_chart(&fresh) {
            Ok(svg) if svg.starts_with("<svg") => {}
            _ => return 21,
        }
    }
    let _ = i;
    if fresh.stats().workers == 0 {
        return 22;
    }
    drop(fresh);
    0
}

/// Child side of `forks_never_hang`: a thread renders on a shared runner
/// as fast as it can (and now and then registers a bundle, reads the stats
/// and starts a new runner that loads a built-in) while this thread forks.
/// SURIMONO_FORKS sets the number of forks (default 1,000: about 7 s).
#[test]
#[ignore = "helper: run in a child process by forks_never_hang"]
fn fork_stress_child() {
    let forks: usize = std::env::var("SURIMONO_FORKS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1000);
    let shared = Arc::new(runner());
    let mut owned = Some(runner());
    if let Some(r) = &owned {
        tiny(r, 10).unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let renders = Arc::new(AtomicU64::new(0));
    let busy = {
        let (shared, stop, renders) = (shared.clone(), stop.clone(), renders.clone());
        std::thread::spawn(move || {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                tiny(&shared, 2000).unwrap();
                n += 1;
                renders.store(n, Ordering::Relaxed);
                if n % 4 == 0 {
                    shared.register_bundle("tiny2", TINY, "entry").unwrap();
                    let _ = shared.stats();
                    let _ = format!("{shared:?}");
                }
                #[cfg(feature = "echarts")]
                if n % 64 == 0 {
                    let fresh = runner();
                    bar_chart(&fresh).unwrap();
                }
            }
        })
    };
    const HANG: Duration = Duration::from_secs(30);
    const AT_ONCE: usize = 4;
    let (mut ok, mut hung, mut failed) = (0usize, 0usize, Vec::new());
    let mut running: Vec<(libc::pid_t, Instant)> = Vec::new();
    let mut reap = |running: &mut Vec<(libc::pid_t, Instant)>, keep: usize| {
        while running.len() > keep {
            let mut i = 0;
            while i < running.len() {
                let (pid, started) = running[i];
                let mut status: libc::c_int = 0;
                // SAFETY: `pid` is our own child that we haven't reaped, and
                // `status` is a valid place to write its status.
                let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if r == pid {
                    if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                        ok += 1;
                    } else {
                        failed.push(status);
                    }
                    running.swap_remove(i);
                } else if started.elapsed() > HANG {
                    // SAFETY: as above; it is still ours until reaped.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, &mut status, 0);
                    }
                    hung += 1;
                    running.swap_remove(i);
                } else {
                    i += 1;
                }
            }
            if running.len() > keep {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    for i in 0..forks {
        // SAFETY: fork has no preconditions. The child runs only this
        // process's own code (the allocator is the system's, which is
        // fork-safe; the runners it starts are its own) and leaves through
        // `_exit`, so no destructor or test-harness code of the parent runs
        // twice.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                child(&shared, owned.take(), i)
            }))
            .unwrap_or(2);
            // SAFETY: ends the child at once; see above.
            unsafe { libc::_exit(code) };
        }
        assert!(pid > 0, "fork failed");
        running.push((pid, Instant::now()));
        reap(&mut running, AT_ONCE - 1);
    }
    reap(&mut running, 0);
    stop.store(true, Ordering::Relaxed);
    busy.join().unwrap();
    // The parent's own runners are unaffected.
    if let Some(r) = &owned {
        tiny(r, 10).unwrap();
    }
    println!(
        "FORK-STRESS forks={forks} ok={ok} hung={hung} failed={failed:?} busy_renders={}",
        renders.load(Ordering::Relaxed)
    );
}

/// A fork while another thread renders through an `IslandRunner` (and
/// registers bundles, reads its stats, starts runners that load built-ins)
/// never leaves the child hanging, whatever the child does with the
/// parent's runners. Runs `fork_stress_child` in its own process, so a hang
/// fails the test.
#[test]
fn forks_never_hang() {
    let exe = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(exe)
        .args(["fork_stress_child", "--exact", "--ignored", "--nocapture"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(900) {
            let _ = child.kill();
            panic!("the fork stress run itself hung");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut out).unwrap();
    assert!(status.success(), "{status:?}: {out}");
    let line = out
        .lines()
        .find(|l| l.starts_with("FORK-STRESS "))
        .unwrap_or_else(|| panic!("no result: {out}"));
    println!("{line}");
    assert!(
        line.contains(" hung=0 ") && line.contains(" failed=[] "),
        "{line}"
    );
}
