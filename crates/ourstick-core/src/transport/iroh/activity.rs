//! What readers are doing, seen from the owner: how many are downloading right
//! now, and how fast data leaves. Fed by iroh-blobs' transfer events.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{watch, Notify};
use tokio::time::MissedTickBehavior;

use crate::transport::ReaderActivity;

/// How often the activity is recomputed.
const TICK: Duration = Duration::from_millis(500);
/// The rate is taken over this long, so it does not jump with every chunk,
/// nor fall to nothing while the owner gets files ready or reads a slow
/// disk (in the big-folder test of ADR 0007 a two-second rate read 0 most of the
/// time).
const RATE_WINDOW: Duration = Duration::from_secs(10);
/// A reader told "not ready" waits for its file this long at most
/// before it asks again (its pauses grow to a minute,
/// `download::not_ready::LONGEST_PAUSE`): so long it counts as a reader, and,
/// with nothing sent since, the owner is getting files ready for it.
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
    /// Connections told "not ready" and waiting, since when (see
    /// [`PREPARING`]).
    waiting_since: HashMap<u64, Instant>,
    /// Bytes sent, over all transfers.
    sent: u64,
    /// A reader reached us directly, at least once.
    direct: bool,
    /// A reader stayed on the relay for a while, at least once.
    relay_only: bool,
    /// Connections kept on the relay by a declined local network (ADR 0020).
    declined: HashSet<u64>,
    /// When bytes last left, and when a reader was last told "not ready".
    sent_at: Option<Instant>,
    not_ready_at: Option<Instant>,
}

impl Counters {
    /// Readers downloading at `now`: a transfer in progress, one that ended
    /// less than [`LINGER`] ago, or one told "not ready" less than
    /// [`PREPARING`] ago. Forgets the others.
    fn readers(&mut self, now: Instant) -> usize {
        self.idle_since
            .retain(|_, since| now.duration_since(*since) < LINGER);
        self.waiting_since
            .retain(|_, since| now.duration_since(*since) < PREPARING);
        let waiting = self
            .waiting_since
            .keys()
            .filter(|connection| {
                !self.active.contains_key(connection) && !self.idle_since.contains_key(connection)
            })
            .count();
        self.active.len() + self.idle_since.len() + waiting
    }

    /// Readers wait for files being got ready: the last thing that happened
    /// was a "not ready", not long ago.
    fn preparing(&self, now: Instant) -> bool {
        self.not_ready_at.is_some_and(|told| {
            now.duration_since(told) < PREPARING && self.sent_at.is_none_or(|sent| sent < told)
        })
    }

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
    /// Wakes the ticker, which sleeps while no one downloads.
    wake: Arc<Notify>,
}

impl Activity {
    pub(super) fn new() -> Self {
        Self {
            counters: Arc::default(),
            sender: Arc::new(watch::channel(ReaderActivity::default()).0),
            wake: Arc::default(),
        }
    }

    /// The counters, to change them: the ticker is woken.
    fn counters_changed(&self) -> MutexGuard<'_, Counters> {
        self.wake.notify_one();
        self.counters()
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
        let mut counters = self.counters_changed();
        counters.idle_since.remove(&connection);
        counters.waiting_since.remove(&connection);
        *counters.active.entry(connection).or_default() += 1;
    }

    /// `bytes` more were sent.
    pub(super) fn sent(&self, bytes: u64) {
        self.sent_at(bytes, Instant::now());
    }

    fn sent_at(&self, bytes: u64, now: Instant) {
        let mut counters = self.counters_changed();
        counters.sent += bytes;
        counters.sent_at = Some(now);
    }

    /// A reader on `connection` was told "not ready": it is waiting for a
    /// file to be made ready, and counts as a reader meanwhile.
    pub(super) fn told_not_ready(&self, connection: u64) {
        self.told_not_ready_at(connection, Instant::now());
    }

    fn told_not_ready_at(&self, connection: u64, now: Instant) {
        let mut counters = self.counters_changed();
        counters.not_ready_at = Some(now);
        counters.waiting_since.insert(connection, now);
    }

    /// A reader reached us directly.
    pub(super) fn reached_directly(&self) {
        self.counters_changed().direct = true;
    }

    /// A reader was still on the relay after a while.
    pub(super) fn reached_only_by_relay(&self) {
        self.counters_changed().relay_only = true;
    }

    /// Whether a declined local network keeps the reader on `connection` on
    /// the relay (ADR 0020).
    pub(super) fn local_network_declined(&self, connection: u64, declined: bool) {
        let mut counters = self.counters_changed();
        if declined {
            counters.declined.insert(connection);
        } else {
            counters.declined.remove(&connection);
        }
    }

    /// A transfer on `connection` ended, completed or not.
    pub(super) fn ended(&self, connection: u64) {
        self.ended_at(connection, Instant::now());
    }

    /// The reader's `connection` closed: it no longer counts, not even for
    /// [`LINGER`].
    pub(super) fn closed(&self, connection: u64) {
        let mut counters = self.counters_changed();
        counters.active.remove(&connection);
        counters.idle_since.remove(&connection);
        counters.waiting_since.remove(&connection);
        counters.declined.remove(&connection);
    }

    fn ended_at(&self, connection: u64, now: Instant) {
        let mut counters = self.counters_changed();
        if let Some(active) = counters.active.get_mut(&connection) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                counters.active.remove(&connection);
                counters.idle_since.insert(connection, now);
            }
        }
    }

    /// Recomputes and publishes the activity every tick, until aborted;
    /// asleep while no one downloads, until something happens.
    /// `giving_way` tells whether a disk the transfers read gives way.
    pub(super) async fn report(self, giving_way: Arc<dyn Fn() -> bool + Send + Sync>) {
        let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let now = Instant::now();
            let (readers, sent, only_relayed, preparing, declined) = {
                let mut counters = self.counters();
                (
                    counters.readers(now),
                    counters.sent,
                    counters.only_relayed(),
                    counters.preparing(now),
                    !counters.declined.is_empty(),
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
                giving_way: readers > 0 && giving_way(),
                local_network_declined: readers > 0 && declined,
            };
            let quiet = activity.readers == 0 && activity.bytes_per_second == 0;
            self.sender.send_if_modified(|current| {
                let changed = *current != activity;
                *current = activity;
                changed
            });
            if quiet {
                self.wake.notified().await;
                ticker.reset();
            }
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
#[path = "activity_tests.rs"]
mod tests;
