//! What the round decided for all the fetches in flight (ADR 0016, rule
//! 4): read by each fetch's progress callback.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::relay::RelayLimiter;

/// How often, in bytes received by all fetches, the relay limit is asked.
/// Small next to the limit's burst (half a second: 625 KB at the default
/// 10 Mbps), so small files are held to it too.
pub(super) const LIMIT_SAMPLE: u64 = 64 * 1024;

/// …but at most this often: asking reads the connection's paths, which is
/// not free, and a fast direct transfer would ask thousands of times a
/// second.
pub(super) const LIMIT_EVERY: Duration = Duration::from_millis(20);

/// When the relay limit is asked: every [`LIMIT_SAMPLE`] bytes, at most
/// every [`LIMIT_EVERY`]. Shared by the fetches of one round, or held by one
/// fetch alone.
#[derive(Debug)]
pub(super) struct LimitSampler {
    start: Instant,
    bytes: AtomicU64,
    /// Milliseconds after `start` it was last asked; `u64::MAX` before.
    asked_ms: AtomicU64,
}

impl Default for LimitSampler {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            bytes: AtomicU64::new(0),
            asked_ms: AtomicU64::new(u64::MAX),
        }
    }
}

impl LimitSampler {
    /// Counts `new_bytes`; whether to ask the limit now.
    pub(super) fn due(&self, new_bytes: u64) -> bool {
        let before = self.bytes.fetch_add(new_bytes, Ordering::Relaxed);
        if before / LIMIT_SAMPLE == (before + new_bytes) / LIMIT_SAMPLE {
            return false;
        }
        let now = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX - 1);
        let last = self.asked_ms.load(Ordering::Relaxed);
        let every = u64::try_from(LIMIT_EVERY.as_millis()).unwrap_or(20);
        if last != u64::MAX && now < last.saturating_add(every) {
            return false;
        }
        self.asked_ms
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }
}

/// What the round decided, read by every fetch.
#[derive(Debug, Default)]
pub(super) struct Decision {
    /// Bytes since the round last looked.
    pub(super) bytes: AtomicU64,
    /// When to ask the relay limit, over the bytes of all of them.
    pub(super) sampler: LimitSampler,
    pub(super) stop: AtomicBool,
    pub(super) pause_until: Mutex<Option<Instant>>,
    /// The round's relay limiter (ADR 0006), here while the round polls the
    /// fetches: a fetch asks it as [`LimitSampler`] says, over all of them,
    /// and its pause holds every fetch.
    pub(super) limiter: Mutex<Option<RelayLimiter>>,
}

impl Decision {
    pub(super) fn pause_until(&self) -> MutexGuard<'_, Option<Instant>> {
        self.pause_until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn limiter(&self) -> MutexGuard<'_, Option<RelayLimiter>> {
        self.limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Asks the relay limit how long every fetch pauses, from the relay
    /// bytes `relay_now` of the connection.
    pub(super) fn limit(&self, relay_now: u64) {
        let now = Instant::now();
        let pause = match self.limiter().as_mut() {
            Some(limiter) => limiter.wait(relay_now, now),
            None => return,
        };
        *self.pause_until() = (!pause.is_zero()).then_some(now + pause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_asked_every_64_kb_at_most_every_20_ms() {
        let sampler = LimitSampler::default();
        // The first crossing asks.
        assert!(!sampler.due(LIMIT_SAMPLE - 1));
        assert!(sampler.due(1));
        // A crossing at once after does not: too soon.
        assert!(!sampler.due(LIMIT_SAMPLE));
        std::thread::sleep(LIMIT_EVERY + Duration::from_millis(5));
        // Bytes within one sample do not ask, however late.
        assert!(!sampler.due(1));
        assert!(sampler.due(LIMIT_SAMPLE));
    }
}
