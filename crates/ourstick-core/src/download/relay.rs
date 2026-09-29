//! The relay's speed limit (ADR 0006 v2): data that arrives through a relay is
//! limited to a configured rate; data over a direct path never is. A download
//! never stops and never asks because of the relay.

use std::time::{Duration, Instant};

/// The reader's relay policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPolicy {
    /// Most bits per second taken through a relay; `None` for no limit.
    pub limit_bits_per_second: Option<u64>,
}

impl Default for RelayPolicy {
    /// 10 Mbps, ADR 0006.
    fn default() -> Self {
        Self {
            limit_bits_per_second: Some(10_000_000),
        }
    }
}

/// How much relay data may pass at once before the limit bites, in seconds
/// of the rate.
const BURST_SECONDS: f64 = 0.5;

/// A token bucket over the bytes a connection received through relays.
#[derive(Debug)]
pub(super) struct RelayLimiter {
    bytes_per_second: f64,
    tokens: f64,
    relay_seen: u64,
    at: Instant,
    /// The reader was told that the limit applies, since data last arrived
    /// directly.
    announced: bool,
}

impl RelayLimiter {
    /// A limiter for `policy`, starting from `relay_now` relay bytes already
    /// received on this connection; `None` without a limit.
    pub(super) fn new(policy: &RelayPolicy, relay_now: u64, now: Instant) -> Option<Self> {
        let bits = policy.limit_bits_per_second?;
        let bytes_per_second = (bits as f64 / 8.0).max(1.0);
        Some(Self {
            bytes_per_second,
            tokens: bytes_per_second * BURST_SECONDS,
            relay_seen: relay_now,
            at: now,
            announced: false,
        })
    }

    /// The connection has received `relay_now` relay bytes by `now`: how long
    /// to wait before taking more data. Zero while within the limit, and
    /// always zero while data arrives directly.
    pub(super) fn wait(&mut self, relay_now: u64, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        let burst = self.bytes_per_second * BURST_SECONDS;
        self.tokens = (self.tokens + elapsed * self.bytes_per_second).min(burst);
        self.tokens -= relay_now.saturating_sub(self.relay_seen) as f64;
        self.relay_seen = relay_now.max(self.relay_seen);
        self.at = now;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.bytes_per_second)
        }
    }

    /// Whether to tell the reader now that the limit applies: once each time
    /// data starts to arrive through a relay.
    pub(super) fn announce(&mut self, through_relay: bool) -> bool {
        let first = through_relay && !self.announced;
        self.announced = through_relay;
        first
    }

    pub(super) fn bits_per_second(&self) -> u64 {
        (self.bytes_per_second * 8.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    #[test]
    fn default_is_10_mbps_and_none_means_no_limit() {
        assert_eq!(
            RelayPolicy::default().limit_bits_per_second,
            Some(10_000_000)
        );
        let none = RelayPolicy {
            limit_bits_per_second: None,
        };
        assert!(RelayLimiter::new(&none, 0, Instant::now()).is_none());
    }

    #[test]
    fn relay_bytes_over_the_rate_wait_for_the_excess() {
        let start = Instant::now();
        // 8 Mbps = 1 MB/s, burst 0.5 MB.
        let policy = RelayPolicy {
            limit_bits_per_second: Some(8 * MB),
        };
        let mut limiter = RelayLimiter::new(&policy, 0, start).unwrap();
        assert_eq!(limiter.wait(MB / 2, start), Duration::ZERO);
        // Another 1 MB at once: 1 s of rate over the budget.
        let wait = limiter.wait(MB / 2 + MB, start);
        assert!((wait.as_secs_f64() - 1.0).abs() < 1e-6, "{wait:?}");
        // After waiting that second, nothing more is owed.
        let later = start + Duration::from_secs(1);
        assert_eq!(limiter.wait(MB / 2 + MB, later), Duration::ZERO);
    }

    #[test]
    fn the_limit_is_announced_once_per_stretch_through_a_relay() {
        let mut limiter = RelayLimiter::new(&RelayPolicy::default(), 0, Instant::now()).unwrap();
        assert!(!limiter.announce(false));
        assert!(limiter.announce(true));
        assert!(!limiter.announce(true));
        assert!(!limiter.announce(false));
        assert!(limiter.announce(true), "again after a direct stretch");
    }

    #[test]
    fn direct_data_never_waits() {
        let start = Instant::now();
        let mut limiter = RelayLimiter::new(&RelayPolicy::default(), 5 * MB, start).unwrap();
        // The relay count does not move while data arrives directly.
        for step in 0..100 {
            assert_eq!(
                limiter.wait(5 * MB, start + Duration::from_millis(step)),
                Duration::ZERO
            );
        }
    }
}
