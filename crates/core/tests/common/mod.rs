//! Helpers shared by the core integration tests.
#![allow(dead_code)]

use std::time::Duration;

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
