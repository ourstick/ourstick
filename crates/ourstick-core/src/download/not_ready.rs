//! Waiting for an owner that is still preparing a blob (ADR 0007): not a
//! failure. The reader asks again after a growing pause, up to a minute, and
//! after a few minutes of waiting on one file it reconnects, so it fetches the
//! record again and follows a newer version. It never gives up on this alone.

use std::time::{Duration, Instant};

/// The first pause after "not ready".
pub(super) const FIRST_PAUSE: Duration = Duration::from_millis(250);

/// Pauses double up to this.
pub(super) const LONGEST_PAUSE: Duration = Duration::from_secs(60);

/// After waiting this long on one file, the download reconnects and fetches
/// the record again.
pub(super) const REFETCH_AFTER: Duration = Duration::from_secs(5 * 60);

/// The waiting on one file.
#[derive(Debug)]
pub(super) struct Waiting {
    since: Instant,
    next_pause: Duration,
}

impl Waiting {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            since: now,
            next_pause: FIRST_PAUSE,
        }
    }

    /// How long to pause before asking again, or `None` once it is time to
    /// fetch the record again.
    pub(super) fn next_pause(&mut self, now: Instant) -> Option<Duration> {
        if now.duration_since(self.since) >= REFETCH_AFTER {
            return None;
        }
        let pause = self.next_pause;
        self.next_pause = (pause * 2).min(LONGEST_PAUSE);
        Some(pause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pauses_double_up_to_a_minute_then_the_record_is_fetched_again() {
        let start = Instant::now();
        let mut waiting = Waiting::new(start);
        let pauses: Vec<_> = (0..11)
            .map(|_| waiting.next_pause(start).unwrap())
            .collect();
        assert_eq!(pauses[0], FIRST_PAUSE);
        assert_eq!(pauses[1], FIRST_PAUSE * 2);
        assert_eq!(pauses[10], LONGEST_PAUSE);
        assert!(waiting
            .next_pause(start + REFETCH_AFTER - Duration::from_secs(1))
            .is_some());
        assert_eq!(waiting.next_pause(start + REFETCH_AFTER), None);
    }
}
