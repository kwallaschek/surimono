//! Forking a process that renders through the facade (P1-PKG-12). Puma,
//! Unicorn and Resque fork after loading the app, possibly while another
//! thread is rendering. A `Renderer` built before the fork keeps working in
//! the child (it starts its own pool and text measurer there, see
//! `Renderer`'s docs), and dropping one there never waits on the parent's
//! threads. The islands crate's own fork test (crates/islands/tests/fork.rs)
//! covers the runner underneath; this one covers the facade's per-process
//! slots on top of it, with the built-in ECharts and Vega-Lite renderers.
//!
//! The forking needs `unsafe` (libc's `fork`, `waitpid`, `waitid`, `kill`,
//! `_exit`); this test module is opted in, and every block has a `SAFETY:`
//! comment.
#![cfg(all(target_os = "linux", feature = "echarts", feature = "vega"))]
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use surimono::{Limits, Options, Renderer};

/// A small registered renderer, so most children render quickly; every
/// `FULL_EVERY`th child renders the ECharts and Vega-Lite document instead.
const TINY: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  let s = 0;
  for (let i = 0; i < spec.n; i++) s += i % 7;
  return '<svg xmlns="http://www.w3.org/2000/svg" width="' + ctx.width + '" height="'
    + ctx.height + '"><desc>' + s + '</desc></svg>';
}
"#;

/// How often a child renders the full document (ECharts and Vega-Lite); the
/// others render the tiny island. Starting the built-ins in a child costs
/// about as much as in a new process (a debug build compiles their bundles),
/// so doing it in every child would make this test take minutes.
const FULL_EVERY: usize = 20;

/// How often a child forks a grandchild (Puma's `fork_worker` forks workers
/// from worker 0, itself a fork). The grandchild renders on the same shared
/// renderer after the child has used it, so it starts its pool again from
/// the child's state; every `NESTED_FULL_EVERY`th one renders the full
/// document.
const NESTED_EVERY: usize = 10;
const NESTED_FULL_EVERY: usize = 50;

/// How long a forked process may take before it counts as hung.
const HANG: Duration = Duration::from_secs(30);

const ECHARTS: &str = r#"{
  "animation": false,
  "textStyle": {"fontFamily": "'Noto Sans'"},
  "xAxis": {"type": "category", "data": ["Q1", "Q2", "Q3", "Q4"]},
  "yAxis": {"type": "value"},
  "series": [{"type": "bar", "data": [120, 200, 150, 80]}]
}"#;

const VEGA_LITE: &str = r#"{
  "width": 300, "height": 180, "padding": 4,
  "config": {"font": "Noto Sans"},
  "data": {"values": [
    {"customer": "Baltic Shipping", "total": 58095},
    {"customer": "Delta Pharma", "total": 33733},
    {"customer": "Granite Labs", "total": 33342}
  ]},
  "mark": "bar",
  "encoding": {
    "y": {"field": "customer", "type": "nominal", "sort": "-x"},
    "x": {"field": "total", "type": "quantitative"}
  }
}"#;

fn island(renderer: &str, width: u32, height: u32, spec: &str) -> String {
    format!(
        "<script type=\"application/vnd.surimono+json\" data-renderer=\"{renderer}\" \
         data-width=\"{width}\" data-height=\"{height}\">{spec}</script>"
    )
}

/// A page with an ECharts and a Vega-Lite island.
fn full_document() -> String {
    format!(
        "<!doctype html><html><body><h1>Q3</h1>\n{}\n<p>between</p>\n{}\n</body></html>",
        island("echarts", 320, 200, ECHARTS),
        island("vega-lite", 320, 200, VEGA_LITE)
    )
}

fn tiny_document(n: u32) -> String {
    format!(
        "<p>{}</p>",
        island("tiny", 10, 10, &format!("{{\"n\": {n}}}"))
    )
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn renderer() -> Renderer {
    let font = |name: &str| {
        let path = repo().join("fixtures/fonts").join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let options = Options::new()
        .font(font("NotoSans-Regular.ttf"))
        .font(font("NotoSans-Bold.ttf"))
        .bundle("tiny", TINY, "entry")
        .pool_size(2)
        // Structure, not speed: generous for a debug build on a busy runner.
        .limits(Limits {
            time: Duration::from_secs(60),
            ..Limits::default()
        });
    Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
}

/// Number of root SVGs the facade put in `out`.
fn svgs(out: &str) -> usize {
    out.matches("<svg").count()
}

/// What a forked child checks; 0 when all is well.
fn child(shared: &Renderer, owned: Option<Renderer>, i: usize) -> i32 {
    // The shared renderer works here: it starts this process's own pool.
    if i % FULL_EVERY == 0 {
        match shared.render_islands(&full_document()) {
            Ok(out) if svgs(&out) == 2 && out.contains("<p>between</p>") => {}
            Ok(_) => return 10,
            Err(_) => return 11,
        }
    } else {
        match shared.render_islands(&tiny_document(1000)) {
            Ok(out) if out.contains("<svg") && out.contains("<desc>2997</desc>") => {}
            Ok(_) => return 12,
            Err(_) => return 13,
        }
    }
    // A second render reuses that pool.
    match shared.render_islands(&tiny_document(10)) {
        Ok(out) if out.contains("<svg") => {}
        _ => return 14,
    }
    // A fork of this fork renders too.
    if i % NESTED_EVERY == NESTED_EVERY / 2 {
        let code = grandchild(shared, i % NESTED_FULL_EVERY == NESTED_EVERY / 2);
        if code != 0 {
            return code;
        }
    }
    // Dropping a renderer built before the fork keeps the parent's state
    // instead of joining threads that don't exist here.
    drop(owned);
    0
}

/// Forks from a forked child; the grandchild renders on `shared` and the
/// child waits for it, killing it after [`HANG`]. 0 when all is well.
fn grandchild(shared: &Renderer, full: bool) -> i32 {
    // SAFETY: fork has no preconditions. As in `fork_stress_child`, the
    // grandchild runs only this process's code and leaves through `_exit`.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ok = if full {
                shared
                    .render_islands(&full_document())
                    .is_ok_and(|out| svgs(&out) == 2 && out.contains("<p>between</p>"))
            } else {
                shared
                    .render_islands(&tiny_document(1000))
                    .is_ok_and(|out| out.contains("<desc>2997</desc>"))
            };
            if ok { 0 } else { 20 }
        }))
        .unwrap_or(21);
        // SAFETY: ends the grandchild at once; see above.
        unsafe { libc::_exit(code) };
    }
    if pid < 0 {
        return 22;
    }
    let started = Instant::now();
    loop {
        let mut status: libc::c_int = 0;
        // SAFETY: `pid` is our own unreaped child; `status` is writable.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r == pid {
            return if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                0
            } else {
                23
            };
        }
        if r < 0 {
            return 24;
        }
        if started.elapsed() > HANG {
            // SAFETY: as above; it is still ours until reaped.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, &mut status, 0);
            }
            return 25;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Child side of `forks_never_hang`: a thread renders the ECharts and
/// Vega-Lite document on a shared renderer as fast as it can while this
/// thread forks. SURIMONO_FORKS sets the number of forks (default 1,000).
#[test]
#[ignore = "helper: run in a child process by forks_never_hang"]
fn fork_stress_child() {
    let forks: usize = std::env::var("SURIMONO_FORKS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1000);
    let shared = Arc::new(renderer());
    let full = full_document();
    let first = shared
        .render_islands(&full)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(svgs(&first), 2, "{first}");
    let mut owned = Some(renderer());
    if let Some(r) = &owned {
        r.render_islands(&tiny_document(10))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let renders = Arc::new(AtomicU64::new(0));
    let busy = {
        let (shared, stop, renders) = (shared.clone(), stop.clone(), renders.clone());
        let (full, first) = (full.clone(), first.clone());
        std::thread::spawn(move || {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let out = shared
                    .render_islands(&full)
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(out, first, "the same output every time");
                n += 1;
                renders.store(n, Ordering::Relaxed);
            }
        })
    };
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
    let started = Instant::now();
    for i in 0..forks {
        // SAFETY: fork has no preconditions. The child runs only this
        // process's own code (the allocator is the system's, which is
        // fork-safe; the pool it starts is its own) and leaves through
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
    busy.join()
        .unwrap_or_else(|_| panic!("the busy thread panicked"));
    // The parent's own renderers are unaffected.
    assert_eq!(
        shared.render_islands(&full).ok().as_deref(),
        Some(first.as_str())
    );
    if let Some(r) = &owned {
        assert!(r.render_islands(&tiny_document(10)).is_ok());
    }
    println!(
        "FORK-STRESS forks={forks} ok={ok} hung={hung} failed={failed:?} busy_renders={} secs={}",
        renders.load(Ordering::Relaxed),
        started.elapsed().as_secs()
    );
}

/// A fork while another thread renders through a `Renderer` never leaves
/// the child hanging: the child renders on the same renderer and drops
/// another one built before the fork, and some children fork a grandchild
/// that renders too. Runs `fork_stress_child` in its own process group, so
/// a hang fails the test and the timeout kills the helper together with
/// every child and grandchild it forked.
#[test]
fn forks_never_hang() {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let mut child = std::process::Command::new(exe)
        .args(["fork_stress_child", "--exact", "--ignored", "--nocapture"])
        .stdout(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    // Read while waiting: a helper that writes more than the pipe holds
    // (64 KiB on Linux) would otherwise block and look hung.
    let mut stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("stdout is piped"));
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        String::from_utf8_lossy(&out).into_owned()
    });
    let group = libc::pid_t::try_from(child.id()).unwrap_or_else(|e| panic!("{e}"));
    // Wait for the helper to exit without reaping it (WNOWAIT): while it is
    // an unreaped zombie its pid, which is also its process group's id,
    // can't be reused, so the kill below can only reach the helper's own
    // group. Reaping first would leave a stale group id.
    let started = Instant::now();
    let exited = loop {
        // SAFETY: an all-zero siginfo_t is a valid value (plain integers and
        // unions of them).
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waitid only writes into `info`, which we own and which
        // lives across the call; `group` is our own unreaped child's pid.
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                libc::id_t::try_from(group).unwrap_or_else(|e| panic!("{e}")),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if r != 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // Fail only after the group kill below, so the helper can't
            // outlive the test.
            break Err(format!("waitid: {e}"));
        }
        // SAFETY: waitid succeeded and filled `info` for a WEXITED query;
        // si_pid is 0 when the helper hasn't exited yet (WNOHANG).
        if unsafe { info.si_pid() } != 0 {
            break Ok(());
        }
        if started.elapsed() > Duration::from_secs(900) {
            break Err("the fork stress run itself hung; killed its process group".to_owned());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // End whatever the helper forked and left behind, so the reader sees end
    // of file even if a stray grandchild still holds the pipe; after a
    // timeout or a waitid error this also ends the helper.
    // SAFETY: kill has no memory-safety preconditions. The helper leads its
    // own process group (process_group(0)), whose id is its pid, and it isn't
    // reaped yet, so the group holds only the helper and its descendants.
    unsafe { libc::kill(-group, libc::SIGKILL) };
    let status = child.wait().unwrap_or_else(|e| panic!("{e}"));
    if let Err(e) = exited {
        panic!("{e}");
    }
    let out = reader
        .join()
        .unwrap_or_else(|_| panic!("the stdout reader panicked"));
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
