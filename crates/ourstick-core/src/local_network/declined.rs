//! When a declined permission is what keeps a peer on the relay (ADR 0020,
//! rule 5): the peer on this computer's network, the path to it the relay,
//! and the probe saying declined, all three for `DECLINED_AFTER`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Access, LocalNetwork};
use crate::blocking::run_blocking;

/// How long the three must hold before it is said: finding a direct path
/// takes a few seconds.
pub const DECLINED_AFTER: Duration = Duration::from_secs(10);
/// How often the probe is asked at most, and the facts looked at.
pub const LOOK_EVERY: Duration = Duration::from_secs(10);

/// Whether, for one peer, a declined permission is what keeps it on the
/// relay, over time.
#[derive(Debug, Default)]
pub struct DeclinedHere {
    /// Since when the three facts have held.
    since: Option<Instant>,
}

impl DeclinedHere {
    /// The facts as they are `now`: whether it is to be said.
    pub fn check(&mut self, same_network: bool, relay: bool, access: Access, now: Instant) -> bool {
        if same_network && relay && access == Access::Declined {
            let since = *self.since.get_or_insert(now);
            now.duration_since(since) >= DECLINED_AFTER
        } else {
            self.since = None;
            false
        }
    }
}

/// A probe shared by every peer, asked at most every `LOOK_EVERY`.
pub struct SharedProbe {
    probe: Arc<dyn LocalNetwork>,
    last: Mutex<Option<(Instant, Access)>>,
}

impl SharedProbe {
    pub fn new(probe: Arc<dyn LocalNetwork>) -> Self {
        Self {
            probe,
            last: Mutex::new(None),
        }
    }

    /// What the probe says, asked again if its answer is older than
    /// `LOOK_EVERY`; on a blocking thread.
    pub async fn access(&self) -> Access {
        let cached = *self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, access)) = cached {
            if at.elapsed() < LOOK_EVERY {
                return access;
            }
        }
        let probe = self.probe.clone();
        let access = run_blocking(move || probe.access()).await;
        *self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((Instant::now(), access));
        access
    }
}

impl std::fmt::Debug for SharedProbe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedProbe")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn it_is_said_once_the_three_have_held_long_enough() {
        let start = Instant::now();
        let mut declined = DeclinedHere::default();
        assert!(!declined.check(true, true, Access::Declined, start));
        assert!(!declined.check(true, true, Access::Declined, start + Duration::from_secs(9)));
        assert!(declined.check(true, true, Access::Declined, start + DECLINED_AFTER));
    }

    #[test]
    fn any_of_the_three_failing_starts_it_again() {
        let start = Instant::now();
        let later = |seconds| start + Duration::from_secs(seconds);
        for (same_network, relay, access) in [
            (false, true, Access::Declined),
            (true, false, Access::Declined),
            (true, true, Access::NotDeclined),
            (true, true, Access::Unknown),
        ] {
            let mut declined = DeclinedHere::default();
            assert!(!declined.check(true, true, Access::Declined, start));
            assert!(!declined.check(same_network, relay, access, later(10)));
            // Held again from here, not from the start.
            assert!(!declined.check(true, true, Access::Declined, later(11)));
            assert!(declined.check(true, true, Access::Declined, later(21)));
        }
    }

    struct Counting(AtomicUsize);

    impl LocalNetwork for Counting {
        fn access(&self) -> Access {
            self.0.fetch_add(1, Ordering::Relaxed);
            Access::Declined
        }
    }

    #[tokio::test]
    async fn the_probe_is_asked_at_most_every_ten_seconds() {
        let counting = Arc::new(Counting(AtomicUsize::new(0)));
        let shared = SharedProbe::new(counting.clone());
        assert_eq!(shared.access().await, Access::Declined);
        assert_eq!(shared.access().await, Access::Declined);
        assert_eq!(counting.0.load(Ordering::Relaxed), 1);
    }
}
