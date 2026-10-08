//! A connection that lost its direct path can stay on the relay although a
//! direct one exists again: seen when a reader changed networks mid-download
//! (phone hotspot to home Wi-Fi, owner on that same Wi-Fi), when the
//! connection kept going through the relay at its limit while a new one went
//! direct at once. So a download that stays on the relay connects again now
//! and then to look for a direct path; it resumes where it was. Never a
//! stop (ADR 0006): only a new look, less often each time it finds none.

use std::time::{Duration, Instant};

/// On the relay this long: connect again to look for a direct path.
pub(super) const FIRST_LOOK_AFTER: Duration = Duration::from_secs(60);
/// The wait doubles after each look that found none, up to this.
pub(super) const LONGEST_WAIT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
pub(super) struct RelayWatch {
    on_relay_since: Option<Instant>,
    wait: Duration,
    /// Whether a declined local network was last told (ADR 0020).
    declined: bool,
}

impl Default for RelayWatch {
    fn default() -> Self {
        Self {
            on_relay_since: None,
            wait: FIRST_LOOK_AFTER,
            declined: false,
        }
    }
}

impl RelayWatch {
    /// Whether a declined local network keeps the data on the relay `now`:
    /// `Some` when that is not what was last told.
    pub(super) fn declined(&mut self, now: bool) -> Option<bool> {
        (now != self.declined).then(|| {
            self.declined = now;
            now
        })
    }

    /// Takes in where data came from at `now`; true when it is time to
    /// connect again.
    pub(super) fn on_check(&mut self, relay: bool, now: Instant) -> bool {
        if !relay {
            self.on_relay_since = None;
            self.wait = FIRST_LOOK_AFTER;
            return false;
        }
        let since = *self.on_relay_since.get_or_insert(now);
        now.duration_since(since) >= self.wait
    }

    /// A new connection is being made: the next look, if the relay stays,
    /// comes later.
    pub(super) fn looked(&mut self) {
        self.on_relay_since = None;
        self.wait = (self.wait * 2).min(LONGEST_WAIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_long_on_the_relay_looks_again_less_often_each_time() {
        let start = Instant::now();
        let mut watch = RelayWatch::default();
        assert!(!watch.on_check(true, start));
        assert!(!watch.on_check(true, start + Duration::from_secs(59)));
        assert!(watch.on_check(true, start + FIRST_LOOK_AFTER));
        watch.looked();
        // Still on the relay after the new connection: twice as long.
        let again = start + Duration::from_secs(100);
        assert!(!watch.on_check(true, again));
        assert!(!watch.on_check(true, again + FIRST_LOOK_AFTER));
        assert!(watch.on_check(true, again + FIRST_LOOK_AFTER * 2));
        for _ in 0..10 {
            watch.looked();
        }
        assert_eq!(watch.wait, LONGEST_WAIT);
    }

    #[test]
    fn a_direct_path_resets_the_wait() {
        let start = Instant::now();
        let mut watch = RelayWatch::default();
        watch.on_check(true, start);
        watch.looked();
        assert!(!watch.on_check(false, start + Duration::from_secs(5)));
        assert!(!watch.on_check(true, start + Duration::from_secs(10)));
        // Continuously on the relay again for the first wait only.
        assert!(watch.on_check(true, start + Duration::from_secs(10) + FIRST_LOOK_AFTER));
    }

    #[test]
    fn a_declined_local_network_is_told_when_it_changes() {
        let mut watch = RelayWatch::default();
        assert_eq!(watch.declined(false), None, "not declined at first");
        assert_eq!(watch.declined(true), Some(true));
        assert_eq!(watch.declined(true), None);
        assert_eq!(watch.declined(false), Some(false));
    }
}
