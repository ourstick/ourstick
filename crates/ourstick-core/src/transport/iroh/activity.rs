//! What readers are doing, seen from the owner: how many are downloading right
//! now, and how fast data leaves. Fed by iroh-blobs' transfer events.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::transport::ReaderActivity;

/// How often the activity is recomputed.
const TICK: Duration = Duration::from_millis(500);
/// The rate is taken over this long, so it does not jump with every chunk.
const RATE_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
struct Counters {
    /// Transfers in progress, per connection.
    active: HashMap<u64, usize>,
    /// Bytes sent, over all transfers.
    sent: u64,
    /// A reader reached us directly, at least once.
    direct: bool,
    /// A reader stayed on the relay for a while, at least once.
    relay_only: bool,
}

impl Counters {
    /// Readers came only through the relay: one stayed on it for a while,
    /// and none came directly.
    fn only_relayed(&self) -> bool {
        self.relay_only && !self.direct
    }
}

/// Shared by the event handler (which counts) and the ticker (which reports).
#[derive(Debug, Clone)]
pub(super) struct Activity {
    counters: Arc<Mutex<Counters>>,
    sender: Arc<watch::Sender<ReaderActivity>>,
}

impl Activity {
    pub(super) fn new() -> Self {
        Self {
            counters: Arc::default(),
            sender: Arc::new(watch::channel(ReaderActivity::default()).0),
        }
    }

    fn counters(&self) -> MutexGuard<'_, Counters> {
        // Every update is a few arithmetic steps; a panic cannot leave them
        // half done in a way that matters for a display.
        self.counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<ReaderActivity> {
        self.sender.subscribe()
    }

    /// A transfer started on `connection`.
    pub(super) fn started(&self, connection: u64) {
        *self.counters().active.entry(connection).or_default() += 1;
    }

    /// `bytes` more were sent.
    pub(super) fn sent(&self, bytes: u64) {
        self.counters().sent += bytes;
    }

    /// A reader reached us directly.
    pub(super) fn reached_directly(&self) {
        self.counters().direct = true;
    }

    /// A reader was still on the relay after a while.
    pub(super) fn reached_only_by_relay(&self) {
        self.counters().relay_only = true;
    }

    /// A transfer on `connection` ended, completed or not.
    pub(super) fn ended(&self, connection: u64) {
        let mut counters = self.counters();
        if let Some(active) = counters.active.get_mut(&connection) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                counters.active.remove(&connection);
            }
        }
    }

    /// Recomputes and publishes the activity every tick, until aborted.
    pub(super) async fn report(self) {
        let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
        let mut ticker = tokio::time::interval(TICK);
        loop {
            ticker.tick().await;
            let now = Instant::now();
            let (readers, sent, only_relayed) = {
                let counters = self.counters();
                (
                    counters.active.len(),
                    counters.sent,
                    counters.only_relayed(),
                )
            };
            samples.push_back((now, sent));
            while samples
                .front()
                .is_some_and(|(at, _)| now.duration_since(*at) > RATE_WINDOW)
            {
                samples.pop_front();
            }
            let activity = ReaderActivity {
                readers,
                bytes_per_second: rate(&samples),
                only_relayed,
            };
            self.sender.send_if_modified(|current| {
                let changed = *current != activity;
                *current = activity;
                changed
            });
        }
    }
}

/// Bytes per second between the oldest and newest sample.
fn rate(samples: &VecDeque<(Instant, u64)>) -> u64 {
    let (Some((first_at, first)), Some((last_at, last))) = (samples.front(), samples.back()) else {
        return 0;
    };
    let seconds = last_at.duration_since(*first_at).as_secs_f64();
    if seconds <= 0.0 {
        return 0;
    }
    ((last - first) as f64 / seconds) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_are_connections_with_a_transfer_in_progress() {
        let activity = Activity::new();
        activity.started(1);
        activity.started(1);
        activity.started(2);
        assert_eq!(activity.counters().active.len(), 2);
        activity.ended(1);
        activity.ended(2);
        assert_eq!(activity.counters().active.len(), 1);
        activity.ended(1);
        activity.ended(1); // an extra end does no harm
        assert!(activity.counters().active.is_empty());
    }

    #[test]
    fn only_relayed_until_someone_comes_directly() {
        let only_relayed = |activity: &Activity| activity.counters().only_relayed();
        let activity = Activity::new();
        assert!(!only_relayed(&activity), "no reader yet");
        activity.reached_only_by_relay();
        assert!(only_relayed(&activity));
        activity.reached_directly();
        assert!(!only_relayed(&activity), "one direct reader is enough");
        activity.reached_only_by_relay();
        assert!(!only_relayed(&activity));
    }

    #[test]
    fn rate_is_over_the_samples_kept() {
        let start = Instant::now();
        let samples: VecDeque<_> = [
            (start, 0),
            (start + Duration::from_millis(500), 1_000_000),
            (start + Duration::from_secs(2), 4_000_000),
        ]
        .into();
        assert_eq!(rate(&samples), 2_000_000);
        assert_eq!(rate(&VecDeque::new()), 0);
        assert_eq!(rate(&[(start, 5)].into()), 0);
    }
}
