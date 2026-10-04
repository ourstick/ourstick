//! What the round decided for all the fetches in flight (ADR 0016, rule
//! 4): read by each fetch's progress callback.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use super::relay::RelayLimiter;

/// How often, in bytes received by all fetches, the relay limit is asked.
/// Small next to the limit's burst (half a second: 625 KB at the default
/// 10 Mbps), so small files are held to it too.
pub(super) const LIMIT_SAMPLE: u64 = 64 * 1024;

/// What the round decided, read by every fetch.
#[derive(Debug, Default)]
pub(super) struct Decision {
    /// Bytes since the round last looked.
    pub(super) bytes: AtomicU64,
    /// Bytes in all, for asking the relay limit every [`LIMIT_SAMPLE`].
    pub(super) sampled: AtomicU64,
    pub(super) stop: AtomicBool,
    pub(super) pause_until: Mutex<Option<Instant>>,
    /// The round's relay limiter (ADR 0006), here while the round polls the
    /// fetches: a fetch asks it every [`LIMIT_SAMPLE`] bytes of all of them,
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
