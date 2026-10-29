//! Copyright (c) Scott A Dixon
//! The time durable journals judge expiry by.
use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;

/// The time a durable conversation or tool-approval journal judges expiry by. Production code
/// passes [`SystemClock`]; a test passes a clock it advances.
pub trait ExpiryClock: Send + Sync {
    /// Milliseconds since the Unix epoch, or `None` before the epoch or beyond `u64`.
    fn unix_ms(&self) -> Option<u64>;
    /// A monotonic instant, so that expiry holds when the wall clock is set back.
    fn instant(&self) -> Instant;
}

/// The system's wall clock and Tokio's monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl ExpiryClock for SystemClock {
    fn unix_ms(&self) -> Option<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis()
            .try_into()
            .ok()
    }
    fn instant(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that moves only when advanced, for testing expiry.
#[derive(Debug)]
pub struct ManualClock {
    start_ms: u64,
    start: Instant,
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    /// A clock that reads `unix_ms` until advanced.
    pub fn new(unix_ms: u64) -> Self {
        Self {
            start_ms: unix_ms,
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        }
    }
    /// Move both readings forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut elapsed = self.elapsed.lock().unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed.saturating_add(by);
    }
    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ExpiryClock for ManualClock {
    fn unix_ms(&self) -> Option<u64> {
        self.start_ms
            .checked_add(u64::try_from(self.elapsed().as_millis()).ok()?)
    }
    fn instant(&self) -> Instant {
        self.start + self.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_clock_moves_both_readings_only_when_advanced() {
        let clock = ManualClock::new(1_000);
        let (unix_ms, instant) = (clock.unix_ms(), clock.instant());
        assert_eq!((clock.unix_ms(), clock.instant()), (unix_ms, instant));
        clock.advance(Duration::from_millis(250));
        assert_eq!(clock.unix_ms(), Some(1_250));
        assert_eq!(clock.instant(), instant + Duration::from_millis(250));
    }

    #[test]
    fn a_manual_clock_beyond_u64_milliseconds_has_no_unix_time() {
        let clock = ManualClock::new(u64::MAX);
        clock.advance(Duration::from_millis(1));
        assert_eq!(clock.unix_ms(), None);
    }
}
