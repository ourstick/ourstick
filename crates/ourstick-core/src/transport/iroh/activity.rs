//! What readers are doing, seen from the owner: how many are downloading right
//! now, and how fast data leaves. Fed by iroh-blobs' transfer events.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::transport::ReaderActivity;

/// How often the activity is recomputed.
const TICK: Duration = Duration::from_millis(500);
/// The rate is taken over this long, so it does not jump with every chunk,
/// nor fall to nothing between suitcases: a big stick is sent in bursts,
/// each suitcase in seconds after tens of seconds getting it ready (the Rescued_Disk
/// test, ADR 0007), and a two-second rate read 0 most of the time.
const RATE_WINDOW: Duration = Duration::from_secs(10);
/// A reader told "not ready" waits for its suitcase this long at most
/// before it asks again (its pauses grow to a minute): so long, with nothing
/// sent since, the owner is getting files ready for it.
const PREPARING: Duration = Duration::from_secs(70);
/// A reader whose last transfer ended counts as downloading this much longer:
/// between two files it decrypts the last one into place, seconds for a big
/// one, and "nobody, someone, nobody" at every file says nothing true.
const LINGER: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct Counters {
    /// Transfers in progress, per connection.
    active: HashMap<u64, usize>,
    /// Connections with no transfer in progress, since when (see [`LINGER`]).
    idle_since: HashMap<u64, Instant>,
    /// Bytes sent, over all transfers.
    sent: u64,
    /// A reader reached us directly, at least once.
    direct: bool,
    /// A reader stayed on the relay for a while, at least once.
    relay_only: bool,
    /// When bytes last left, and when a reader was last told "not ready".
    sent_at: Option<Instant>,
    not_ready_at: Option<Instant>,
}

impl Counters {
    /// Readers downloading at `now`: a transfer in progress, or one that
    /// ended less than [`LINGER`] ago. Forgets the ones idle for longer.
    fn readers(&mut self, now: Instant) -> usize {
        self.idle_since
            .retain(|_, since| now.duration_since(*since) < LINGER);
        self.active.len() + self.idle_since.len()
    }

    /// Readers came only through the relay: one stayed on it for a while,
    /// and none came directly.
    /// Readers wait for files being got ready: the last thing that happened
    /// was a "not ready", not long ago.
    fn preparing(&self, now: Instant) -> bool {
        self.not_ready_at.is_some_and(|told| {
            now.duration_since(told) < PREPARING && self.sent_at.is_none_or(|sent| sent < told)
        })
    }

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
        let mut counters = self.counters();
        counters.idle_since.remove(&connection);
        *counters.active.entry(connection).or_default() += 1;
    }

    /// `bytes` more were sent.
    pub(super) fn sent(&self, bytes: u64) {
        self.sent_at(bytes, Instant::now());
    }

    fn sent_at(&self, bytes: u64, now: Instant) {
        let mut counters = self.counters();
        counters.sent += bytes;
        counters.sent_at = Some(now);
    }

    /// A reader on `connection` was told "not ready": it is waiting for a
    /// suitcase, and counts as a reader meanwhile.
    pub(super) fn told_not_ready(&self, connection: u64) {
        self.told_not_ready_at(connection, Instant::now());
    }

    fn told_not_ready_at(&self, connection: u64, now: Instant) {
        let mut counters = self.counters();
        counters.not_ready_at = Some(now);
        if !counters.active.contains_key(&connection) {
            counters.idle_since.insert(connection, now);
        }
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
        self.ended_at(connection, Instant::now());
    }

    /// The reader's `connection` closed: it no longer counts, not even for
    /// [`LINGER`].
    pub(super) fn closed(&self, connection: u64) {
        let mut counters = self.counters();
        counters.active.remove(&connection);
        counters.idle_since.remove(&connection);
    }

    fn ended_at(&self, connection: u64, now: Instant) {
        let mut counters = self.counters();
        if let Some(active) = counters.active.get_mut(&connection) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                counters.active.remove(&connection);
                counters.idle_since.insert(connection, now);
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
            let (readers, sent, only_relayed, preparing) = {
                let mut counters = self.counters();
                (
                    counters.readers(now),
                    counters.sent,
                    counters.only_relayed(),
                    counters.preparing(now),
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
                preparing: readers > 0 && preparing,
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
    fn a_reader_between_two_files_still_counts() {
        let start = Instant::now();
        let activity = Activity::new();
        activity.started(1);
        activity.ended_at(1, start);
        let readers = |at: Duration| activity.counters().readers(start + at);
        assert_eq!(readers(Duration::from_secs(5)), 1);
        // The next file: still one reader.
        activity.started(1);
        assert_eq!(readers(Duration::from_secs(6)), 1);
        activity.ended_at(1, start + Duration::from_secs(10));
        assert_eq!(readers(LINGER), 1);
        // Idle for longer than LINGER: gone.
        assert_eq!(readers(LINGER + Duration::from_secs(10)), 0);
        assert!(activity.counters().idle_since.is_empty());
    }

    #[test]
    fn a_closed_connection_stops_counting_at_once() {
        let start = Instant::now();
        let activity = Activity::new();
        activity.started(1);
        activity.started(2);
        activity.ended_at(1, start);
        activity.closed(1);
        activity.closed(2);
        assert_eq!(activity.counters().readers(start), 0);
    }

    #[test]
    fn a_reader_told_not_ready_is_waiting_for_files_got_ready() {
        let start = Instant::now();
        let activity = Activity::new();
        let at = |seconds| start + Duration::from_secs(seconds);
        assert!(!activity.counters().preparing(at(0)));
        // Told "not ready": a reader, waiting while its files are got ready.
        activity.told_not_ready_at(7, at(1));
        assert_eq!(activity.counters().readers(at(2)), 1);
        assert!(activity.counters().preparing(at(2)));
        // Bytes leave: no longer only waiting.
        activity.sent_at(1000, at(3));
        assert!(!activity.counters().preparing(at(4)));
        // Told again later, and waiting as long as a reader's longest pause.
        activity.told_not_ready_at(7, at(5));
        assert!(activity
            .counters()
            .preparing(at(5) + PREPARING - Duration::from_secs(1)));
        assert!(!activity.counters().preparing(at(5) + PREPARING));
        // A transfer in progress on that connection is not made idle.
        activity.started(8);
        activity.told_not_ready_at(8, at(6));
        assert_eq!(activity.counters().active.get(&8), Some(&1));
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
