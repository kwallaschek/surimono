//! What a `fork` does around work marked `busy`. Its own process, one test at a
//! time: the guard is process-wide.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use surimono_core::{busy, paused};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn alone() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a `fork` of this process takes in the parent.
fn fork_takes() -> Duration {
    let started = Instant::now();
    // SAFETY: fork has no preconditions, and the child leaves at once through
    // `_exit`, running nothing of the parent's.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: ends the child; see above.
        unsafe { libc::_exit(0) };
    }
    let took = started.elapsed();
    assert!(pid > 0, "fork failed");
    let mut status = 0;
    // SAFETY: `pid` is our own child, and `status` is a valid place to write.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    took
}

/// A thread that is busy for `held`, once started: `entered` says when.
fn busy_for(held: Duration) -> (mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
    let (tell, entered) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _busy = busy().unwrap_or_else(|e| panic!("{e}"));
        let _ = tell.send(());
        std::thread::sleep(held);
    });
    entered.recv().unwrap_or_else(|e| panic!("{e}"));
    (entered, worker)
}

#[test]
fn a_fork_waits_for_running_work() {
    let _alone = alone();
    let held = Duration::from_millis(400);
    let (_, worker) = busy_for(held);
    let took = fork_takes();
    assert!(took > held / 2, "took {took:?}");
    worker
        .join()
        .unwrap_or_else(|_| panic!("the worker panicked"));
    assert!(fork_takes() < Duration::from_secs(1));
}

#[test]
fn a_fork_gives_up_after_five_seconds() {
    let _alone = alone();
    let (_, worker) = busy_for(Duration::from_secs(7));
    let took = fork_takes();
    assert!(took >= Duration::from_millis(4900), "took {took:?}");
    assert!(took < Duration::from_millis(6500), "took {took:?}");
    worker
        .join()
        .unwrap_or_else(|_| panic!("the worker panicked"));
}

#[test]
fn work_started_during_a_fork_is_held_back_and_then_runs() {
    let _alone = alone();
    let held = Duration::from_millis(600);
    let (_, first) = busy_for(held);
    let started = Instant::now();
    let forking = std::thread::spawn(fork_takes);
    std::thread::sleep(Duration::from_millis(100));
    let (tell, entered) = mpsc::channel();
    let second = std::thread::spawn(move || {
        let busy = busy().unwrap_or_else(|e| panic!("{e}"));
        let _ = tell.send(started.elapsed());
        drop(busy);
    });
    let waited = entered
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(waited > held / 2, "entered after {waited:?}");
    first.join().unwrap_or_else(|_| panic!("a worker panicked"));
    second
        .join()
        .unwrap_or_else(|_| panic!("a worker panicked"));
    forking
        .join()
        .unwrap_or_else(|_| panic!("the fork panicked"));
}

#[test]
fn a_fork_doesnt_wait_for_paused_work() {
    let _alone = alone();
    let (tell, entered) = mpsc::channel();
    let (go_on, wait) = mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        let _busy = busy().unwrap_or_else(|e| panic!("{e}"));
        paused(|| {
            let _ = tell.send(());
            let _ = wait.recv();
        });
    });
    entered.recv().unwrap_or_else(|e| panic!("{e}"));
    let took = fork_takes();
    assert!(took < Duration::from_secs(1), "took {took:?}");
    drop(go_on);
    worker
        .join()
        .unwrap_or_else(|_| panic!("the worker panicked"));
}
