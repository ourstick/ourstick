//! Time, behind a trait so tests control it (CLAUDE.md rule 2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of the current time.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch. Never before the epoch (clamped to 0).
    fn now_ms(&self) -> u64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to, for tests.
#[derive(Debug, Default)]
pub struct FixedClock {
    now_ms: AtomicU64,
}

impl FixedClock {
    pub fn new(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    pub fn set(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_after_2026() {
        assert!(SystemClock.now_ms() > 1_767_225_600_000);
    }

    #[test]
    fn fixed_clock_moves_only_when_set() {
        let clock = FixedClock::new(5);
        assert_eq!(clock.now_ms(), 5);
        clock.set(9);
        assert_eq!(clock.now_ms(), 9);
        assert_eq!(FixedClock::default().now_ms(), 0);
    }
}
