//! The one place that reads the clocks and waits. Everything else gets the
//! time handed in, so tests control it.

use std::time::{Duration, Instant};

use jiff::{Timestamp, Zoned};

/// The clocks and waiting on them.
pub(crate) trait Clock {
    /// The monotonic clock. It stands still while the Mac sleeps, so it
    /// measures durations within one run only.
    fn now(&self) -> Instant;
    /// The wall clock, which runs on while the Mac sleeps.
    fn timestamp(&self) -> Timestamp;
    /// Blocks the calling thread for `pause`.
    fn sleep(&self, pause: Duration);
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the one place that reads the clock"
    )]
    fn now(&self) -> Instant {
        Instant::now()
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the one place that reads the clock"
    )]
    fn timestamp(&self) -> Timestamp {
        Timestamp::now()
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the one place that waits"
    )]
    fn sleep(&self, pause: Duration) {
        std::thread::sleep(pause);
    }
}

/// The local time of day, for what the user reads: log lines and timers.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the clock module is the one place that reads the clock"
)]
pub(crate) fn wall() -> Zoned {
    Zoned::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_runs() {
        let start = SystemClock.now();
        SystemClock.sleep(Duration::from_millis(20));
        assert!(
            SystemClock.now().duration_since(start) >= Duration::from_millis(20),
            "sleeping takes time"
        );
        assert!(
            wall().year() >= 2026,
            "the local time of day, not a default"
        );
        assert!(
            SystemClock.timestamp().as_second() > 1_790_000_000,
            "the wall clock, not a default"
        );
    }
}
