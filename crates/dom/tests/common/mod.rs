//! Helpers shared by the DOM integration tests.
#![allow(dead_code)]

use std::time::Duration;

use surimono_core::Limits;

/// The test time-scale factor: `SURIMONO_TEST_TIME_SCALE`, an integer of at
/// least 1, default 1. Slow builds (AddressSanitizer in
/// `scripts/ci/asan.sh`) set it so wall-clock limits and bounds in the
/// tests stretch with the build. It never changes the library's defaults;
/// only tests read it. A value that is not an integer of at least 1 fails
/// the test, so a typo can't silently run unscaled.
pub fn time_scale() -> u32 {
    match std::env::var("SURIMONO_TEST_TIME_SCALE") {
        Err(std::env::VarError::NotPresent) => 1,
        Ok(s) if s.is_empty() => 1,
        Ok(s) => match s.parse::<u32>() {
            Ok(n) if n >= 1 => n,
            _ => panic!("SURIMONO_TEST_TIME_SCALE must be an integer of at least 1, not {s:?}"),
        },
        Err(e) => panic!("SURIMONO_TEST_TIME_SCALE: {e}"),
    }
}

/// `d` times [`time_scale`].
pub fn scaled(d: Duration) -> Duration {
    d * time_scale()
}

/// `limits` with the time limit times [`time_scale`]. Every outcome the DOM
/// tests check is the same at any time limit; a slow build must not hit it
/// early and report `TimeLimit` instead.
pub fn scaled_limits(limits: Limits) -> Limits {
    let time = scaled(limits.time);
    limits.with_time(time)
}

/// The default sandbox limits, scaled.
pub fn limits() -> Limits {
    scaled_limits(Limits::default())
}

/// Limits for a cost test, whose answer is a ratio of two runs, not a time:
/// a time limit no build reaches, so a slow one (ASan) can't end the work
/// with `TimeLimit` before it is measured (P2-DOM-17).
pub fn cost_limits() -> Limits {
    Limits::default().with_time(Duration::from_secs(600))
}

/// How much longer `f(big)` takes than `f(small)`, each the fastest of five
/// runs in this process, the two sizes taking turns so that a change in the
/// machine's load reaches both. `f` times only the work it is about; a ratio
/// from one run doesn't depend on how fast the build or the machine is.
pub fn cost_ratio(small: usize, big: usize, mut f: impl FnMut(usize) -> Duration) -> f64 {
    let (mut a, mut b) = (Duration::MAX, Duration::MAX);
    for _ in 0..5 {
        a = a.min(f(small));
        b = b.min(f(big));
    }
    b.as_secs_f64() / a.as_secs_f64().max(1e-6)
}
