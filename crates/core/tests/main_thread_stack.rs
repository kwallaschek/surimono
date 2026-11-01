//! The stack check on the process's main thread. libtest runs tests on
//! threads of its own, so this file has no harness: `main` is the test.
//!
//! On musl, `pthread_getattr_np` reports only the part of the main thread's
//! stack mapped so far, so `engine::stack_low` bounds it by `RLIMIT_STACK`
//! below the top instead (review of #45: that fallback had only been
//! compile-checked). Run with `--target x86_64-unknown-linux-musl` to
//! exercise it; on glibc and macOS the same checks hold through the
//! platform's own answer.

use surimono_core::{Limits, Sandbox, SandboxError};

/// JS stack for the sandbox: far more than a fresh main thread has mapped,
/// and with the headroom well inside the usual 8 MiB `RLIMIT_STACK`.
const STACK: usize = 4 * 1024 * 1024;

fn main() {
    if let Some(why) = skip_reason() {
        println!("main_thread_stack: skipped, {why}");
        return;
    }
    let limits = Limits::default().with_stack_bytes(STACK);
    let mut sb = match Sandbox::new(limits) {
        Ok(sb) => sb,
        Err(e) => panic!("a {STACK}-byte stack limit on the main thread was refused: {e}"),
    };
    // Recursing to the engine's own limit grows the stack into the part
    // not yet mapped; it must end with the typed error, not a crash.
    let depth: u32 = sb
        .eval("let d = 0; function f() { d++; f(); } try { f(); } catch (e) {} d")
        .unwrap();
    assert_eq!(
        sb.eval::<()>("(function g() { g(); })()"),
        Err(SandboxError::StackOverflow)
    );
    // A 512 KiB limit allows about an eighth of that depth; a much smaller
    // ratio would mean the limit isn't what bounds the recursion.
    let mut small = Sandbox::new(Limits::default()).unwrap();
    let small_depth: u32 = small
        .eval("let d = 0; function f() { d++; f(); } try { f(); } catch (e) {} d")
        .unwrap();
    assert!(
        depth > 4 * small_depth,
        "depth {depth} with {STACK} bytes, {small_depth} with the default"
    );
    println!("main_thread_stack: ok, depth {depth} ({small_depth} at the default limit)");
}

/// The checks need a main thread whose stack can hold the limit plus the
/// headroom: a finite `RLIMIT_STACK` of at least 6 MiB. (With an unlimited
/// one, musl keeps the mapped part and refuses, by design.) Read from
/// `/proc/self/limits` on Linux; macOS's main thread has 8 MiB.
fn skip_reason() -> Option<String> {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return Some("no stack check on this platform".into());
    }
    if !cfg!(target_os = "linux") {
        return None;
    }
    let limits = std::fs::read_to_string("/proc/self/limits").ok()?;
    let soft = limits
        .lines()
        .find(|l| l.starts_with("Max stack size"))?
        .split_whitespace()
        .nth(3)?
        .to_owned();
    match soft.parse::<u64>() {
        Ok(n) if n >= 6 * 1024 * 1024 => None,
        Ok(n) => Some(format!("RLIMIT_STACK is only {n} bytes")),
        Err(_) => Some(format!("RLIMIT_STACK is {soft}")),
    }
}
