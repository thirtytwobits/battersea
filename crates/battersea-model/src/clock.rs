//! Copyright (c) Scott A Dixon
//! The time and timers Battersea judges deadlines, backoff and expiry by.
use jiff::{tz::Offset, Timestamp};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{watch, Notify},
    time::Instant,
};

/// A timer future returned by [`Clock::sleep_until`].
pub type Sleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// The host's time source. Production code passes [`SystemClock`]; a test passes a
/// [`ManualClock`] it advances.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch, or `None` before the epoch or beyond `u64`.
    fn unix_ms(&self) -> Option<u64>;
    /// A monotonic instant, so that deadlines and expiry hold when the wall clock is set back.
    fn instant(&self) -> Instant;
    /// Completes once [`Clock::instant`] reaches `deadline`.
    fn sleep_until(&self, deadline: Instant) -> Sleep;
}

/// A deadline elapsed before its future completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

/// `clock.instant() + duration`, or `None` when the sum is not representable.
pub fn deadline_after(clock: &dyn Clock, duration: Duration) -> Option<Instant> {
    clock.instant().checked_add(duration)
}

/// The wall time of `clock`. A clock without a representable Unix time reads the epoch.
pub fn utc_now(clock: &dyn Clock) -> Timestamp {
    clock
        .unix_ms()
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

/// `timestamp` in RFC 3339 at `+00:00`, its fraction of a second in whole milli-, micro- or
/// nanoseconds.
pub fn rfc3339(timestamp: Timestamp) -> String {
    format!(
        "{:.*}",
        fraction_digits(timestamp),
        timestamp.display_with_offset(Offset::UTC)
    )
}

/// The digits of `timestamp`'s fraction of a second: none, or whole milli-, micro- or
/// nanoseconds.
fn fraction_digits(timestamp: Timestamp) -> usize {
    match timestamp.subsec_nanosecond().unsigned_abs() {
        0 => 0,
        nanos if nanos % 1_000_000 == 0 => 3,
        nanos if nanos % 1_000 == 0 => 6,
        _ => 9,
    }
}

/// Serde for an optional [`Timestamp`] as RFC 3339 in UTC, written with `Z` and its fraction of
/// a second in whole milli-, micro- or nanoseconds. It reads any RFC 3339 timestamp.
pub mod optional_utc_rfc3339 {
    use jiff::Timestamp;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct Utc(Timestamp);

    impl Serialize for Utc {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(&format_args!(
                "{:.*}",
                super::fraction_digits(self.0),
                self.0
            ))
        }
    }

    pub fn serialize<S: Serializer>(
        timestamp: &Option<Timestamp>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match timestamp {
            Some(timestamp) => serializer.serialize_some(&Utc(*timestamp)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Timestamp>, D::Error> {
        Option::<Timestamp>::deserialize(deserializer)
    }
}

/// Completes once `duration` has passed on `clock`. An unrepresentable duration never completes.
pub async fn sleep(clock: &dyn Clock, duration: Duration) {
    match deadline_after(clock, duration) {
        Some(deadline) => clock.sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Runs `future` until `deadline` on `clock`. A future that is ready when the deadline is
/// reached completes.
pub async fn timeout_at<F: Future>(
    clock: &dyn Clock,
    deadline: Instant,
    future: F,
) -> Result<F::Output, Elapsed> {
    let timer = clock.sleep_until(deadline);
    tokio::pin!(future);
    tokio::select! {
        biased;
        output = &mut future => Ok(output),
        () = timer => Err(Elapsed),
    }
}

/// Runs `future` for at most `duration` on `clock`.
pub async fn timeout<F: Future>(
    clock: &dyn Clock,
    duration: Duration,
    future: F,
) -> Result<F::Output, Elapsed> {
    match deadline_after(clock, duration) {
        Some(deadline) => timeout_at(clock, deadline, future).await,
        None => Ok(future.await),
    }
}

/// The system's wall clock and Tokio's monotonic clock and timers.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
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
    fn sleep_until(&self, deadline: Instant) -> Sleep {
        Box::pin(tokio::time::sleep_until(deadline))
    }
}

/// A clock that moves only when advanced. Its timers complete when an advance reaches their
/// deadline, and a test can observe which timers are pending.
#[derive(Debug, Clone)]
pub struct ManualClock {
    inner: Arc<ManualInner>,
}

#[derive(Debug)]
struct ManualInner {
    start_ms: u64,
    start: Instant,
    elapsed: watch::Sender<Duration>,
    timers: Mutex<Timers>,
    /// Signals timer observers when a timer registers or the clock advances.
    changed: Notify,
}

#[derive(Debug, Default)]
struct Timers {
    next: u64,
    pending: BTreeMap<u64, Instant>,
}

/// Removes a pending timer when its sleep completes or is dropped.
struct Registration {
    inner: Arc<ManualInner>,
    id: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.inner.timers().pending.remove(&self.id);
        self.inner.changed.notify_waiters();
    }
}

impl ManualInner {
    fn timers(&self) -> std::sync::MutexGuard<'_, Timers> {
        self.timers.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn now(&self) -> Instant {
        self.start + *self.elapsed.borrow()
    }
}

impl ManualClock {
    /// A clock that reads `unix_ms` until advanced.
    pub fn new(unix_ms: u64) -> Self {
        Self {
            inner: Arc::new(ManualInner {
                start_ms: unix_ms,
                start: Instant::now(),
                elapsed: watch::Sender::new(Duration::ZERO),
                timers: Mutex::default(),
                changed: Notify::new(),
            }),
        }
    }
    /// Move both readings forward by `by` and complete every timer the new time reaches.
    pub fn advance(&self, by: Duration) {
        self.inner
            .elapsed
            .send_modify(|elapsed| *elapsed = elapsed.saturating_add(by));
        self.inner.changed.notify_waiters();
    }
    /// The time remaining for each pending timer, shortest first. Elapsed timers read zero.
    pub fn pending_timers(&self) -> Vec<Duration> {
        let now = self.inner.now();
        let mut remaining = self
            .inner
            .timers()
            .pending
            .values()
            .map(|deadline| deadline.saturating_duration_since(now))
            .collect::<Vec<_>>();
        remaining.sort();
        remaining
    }
    /// Waits until a timer that an advance must reach is pending within `window` of the current
    /// time, and returns its remaining time. The wait ends on a signal from a timer
    /// registration, completion or a clock advance, not on a timer. Elapsed timers complete
    /// without an advance and are not reported.
    pub async fn timer_within(&self, window: Duration) -> Duration {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(remaining) = self
                .pending_timers()
                .into_iter()
                .find(|remaining| !remaining.is_zero() && *remaining <= window)
            {
                return remaining;
            }
            changed.await;
        }
    }
    /// Waits until no elapsed timer is pending: every sleep the clock has reached has completed
    /// or been dropped. A sleep completes when its task next polls it, so a test that advances
    /// the clock waits here before reading what the advance caused.
    pub async fn settled(&self) {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .pending_timers()
                .iter()
                .all(|remaining| !remaining.is_zero())
            {
                return;
            }
            changed.await;
        }
    }
    /// Waits for a timer within `window`, then advances exactly to it.
    pub async fn advance_to_timer_within(&self, window: Duration) -> Duration {
        let remaining = self.timer_within(window).await;
        self.advance(remaining);
        remaining
    }
}

impl Clock for ManualClock {
    fn unix_ms(&self) -> Option<u64> {
        let elapsed = u64::try_from(self.inner.elapsed.borrow().as_millis()).ok()?;
        self.inner.start_ms.checked_add(elapsed)
    }
    fn instant(&self) -> Instant {
        self.inner.now()
    }
    fn sleep_until(&self, deadline: Instant) -> Sleep {
        let inner = self.inner.clone();
        Box::pin(async move {
            let mut elapsed = inner.elapsed.subscribe();
            let _registration = {
                let mut timers = inner.timers();
                let id = timers.next;
                timers.next += 1;
                timers.pending.insert(id, deadline);
                Registration {
                    inner: inner.clone(),
                    id,
                }
            };
            inner.changed.notify_waiters();
            while inner.start + *elapsed.borrow_and_update() < deadline {
                // The sender lives as long as `inner`, which this future holds.
                let _ = elapsed.changed().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, Waker};

    fn poll<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

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
    fn wall_time_follows_the_clock() {
        let start = 784_111_777_000;
        let clock = ManualClock::new(start);
        assert_eq!(utc_now(&clock).as_millisecond(), start as i64);
        clock.advance(Duration::from_millis(1_500));
        assert_eq!(utc_now(&clock).as_millisecond(), start as i64 + 1_500);
        assert_eq!(utc_now(&ManualClock::new(u64::MAX)), Timestamp::UNIX_EPOCH);
    }

    /// Times across a second's fractions.
    fn timestamps() -> impl Iterator<Item = Timestamp> {
        [0, 120_000_000, 123_400_000, 123_456_789]
            .into_iter()
            .map(|nanos| Timestamp::new(784_111_777, nanos).unwrap())
    }

    /// The digits after the decimal point of an RFC 3339 `timestamp` ending in `offset`.
    fn fraction(timestamp: &str, offset: &str) -> usize {
        timestamp
            .strip_suffix(offset)
            .and_then(|rest| rest.split_once('.'))
            .map_or(0, |(_, digits)| digits.len())
    }

    #[test]
    fn an_rfc3339_time_is_at_utc_and_reads_back() {
        for timestamp in timestamps() {
            let written = rfc3339(timestamp);
            assert_eq!(
                written.parse::<Timestamp>().unwrap(),
                timestamp,
                "{written}"
            );
            assert!(written.ends_with("+00:00"), "{written}");
            assert!(
                [0, 3, 6, 9].contains(&fraction(&written, "+00:00")),
                "{written}"
            );
        }
    }

    #[test]
    fn a_serialised_optional_time_reads_back() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Expiry(#[serde(with = "optional_utc_rfc3339")] Option<Timestamp>);

        for timestamp in timestamps().map(Some).chain([None]) {
            let json = serde_json::to_value(Expiry(timestamp)).unwrap();
            let written = json.as_str().map(str::to_owned);
            let Expiry(read) = serde_json::from_value(json).unwrap();
            assert_eq!(read, timestamp, "{written:?}");
            if let Some(written) = written {
                assert!(written.ends_with('Z'), "{written}");
                assert!([0, 3, 6, 9].contains(&fraction(&written, "Z")), "{written}");
            }
        }
    }

    #[test]
    fn a_manual_clock_beyond_u64_milliseconds_has_no_unix_time() {
        let clock = ManualClock::new(u64::MAX);
        clock.advance(Duration::from_millis(1));
        assert_eq!(clock.unix_ms(), None);
    }

    #[test]
    fn a_manual_timer_completes_exactly_when_an_advance_reaches_its_deadline() {
        let clock = ManualClock::new(0);
        let mut timer = clock.sleep_until(clock.instant() + Duration::from_millis(100));
        assert!(poll(timer.as_mut()).is_pending());
        assert_eq!(clock.pending_timers(), [Duration::from_millis(100)]);
        clock.advance(Duration::from_millis(99));
        assert!(poll(timer.as_mut()).is_pending());
        assert_eq!(clock.pending_timers(), [Duration::from_millis(1)]);
        clock.advance(Duration::from_millis(1));
        assert!(poll(timer.as_mut()).is_ready());
        assert!(clock.pending_timers().is_empty());
    }

    #[test]
    fn a_dropped_or_elapsed_timer_is_no_longer_pending() {
        let clock = ManualClock::new(0);
        let mut dropped = clock.sleep_until(clock.instant() + Duration::from_millis(5));
        let mut elapsed = clock.sleep_until(clock.instant());
        assert!(poll(dropped.as_mut()).is_pending());
        assert!(poll(elapsed.as_mut()).is_ready());
        assert_eq!(clock.pending_timers(), [Duration::from_millis(5)]);
        drop(dropped);
        assert!(clock.pending_timers().is_empty());
    }

    #[test]
    fn a_deadline_elapses_only_when_the_clock_reaches_it() {
        let clock = ManualClock::new(0);
        let mut bounded = Box::pin(timeout(
            &clock,
            Duration::from_millis(10),
            std::future::pending::<()>(),
        ));
        assert!(poll(bounded.as_mut()).is_pending());
        clock.advance(Duration::from_millis(9));
        assert!(poll(bounded.as_mut()).is_pending());
        clock.advance(Duration::from_millis(1));
        assert_eq!(poll(bounded.as_mut()), Poll::Ready(Err(Elapsed)));
    }

    #[test]
    fn a_future_ready_at_its_deadline_completes() {
        let clock = ManualClock::new(0);
        clock.advance(Duration::from_millis(1));
        let deadline = clock.instant();
        let mut bounded = Box::pin(timeout_at(&clock, deadline, async { 7 }));
        assert_eq!(poll(bounded.as_mut()), Poll::Ready(Ok(7)));
    }

    #[test]
    fn an_unrepresentable_duration_never_elapses() {
        let clock = ManualClock::new(0);
        let mut bounded = Box::pin(timeout(&clock, Duration::MAX, async { 3 }));
        assert_eq!(poll(bounded.as_mut()), Poll::Ready(Ok(3)));
        let mut forever = Box::pin(sleep(&clock, Duration::MAX));
        clock.advance(Duration::from_secs(u32::MAX.into()));
        assert!(poll(forever.as_mut()).is_pending());
    }

    #[test]
    fn an_advance_that_brings_a_timer_into_the_window_wakes_its_observer() {
        let clock = ManualClock::new(0);
        let mut timer = clock.sleep_until(clock.instant() + Duration::from_millis(100));
        assert!(poll(timer.as_mut()).is_pending());
        let mut observer = Box::pin(clock.timer_within(Duration::from_millis(10)));
        assert!(poll(observer.as_mut()).is_pending());
        clock.advance(Duration::from_millis(90));
        assert_eq!(
            poll(observer.as_mut()),
            Poll::Ready(Duration::from_millis(10))
        );
    }

    #[test]
    fn the_clock_settles_once_each_reached_timer_completes_or_is_dropped() {
        let clock = ManualClock::new(0);
        let mut completed = clock.sleep_until(clock.instant() + Duration::from_millis(10));
        let mut dropped = clock.sleep_until(clock.instant() + Duration::from_millis(10));
        assert!(poll(completed.as_mut()).is_pending());
        assert!(poll(dropped.as_mut()).is_pending());
        assert!(poll(Box::pin(clock.settled()).as_mut()).is_ready());
        clock.advance(Duration::from_millis(10));
        let mut settled = Box::pin(clock.settled());
        assert!(poll(settled.as_mut()).is_pending());
        assert!(poll(completed.as_mut()).is_ready());
        assert!(poll(settled.as_mut()).is_pending());
        drop(dropped);
        assert!(poll(settled.as_mut()).is_ready());
    }

    #[tokio::test]
    async fn a_test_can_wait_for_a_timer_and_advance_exactly_to_it() {
        let clock = ManualClock::new(0);
        let sleeper = {
            let clock = clock.clone();
            tokio::spawn(async move { sleep(&clock, Duration::from_millis(40)).await })
        };
        assert_eq!(
            clock
                .advance_to_timer_within(Duration::from_millis(50))
                .await,
            Duration::from_millis(40)
        );
        sleeper.await.unwrap();
        assert_eq!(clock.unix_ms(), Some(40));
    }
}
