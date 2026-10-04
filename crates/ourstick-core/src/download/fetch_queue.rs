//! Small files' blobs fetched several at once (ADR 0016): up to
//! [`FETCHING_AT_ONCE`] on the round's connection, polled together on the
//! round's task. Each fetch only adds its bytes to one count and reads the
//! round's decision (go on, pause until, stop); the round watches the path,
//! the relay limit and the user's stop for all of them, on every landing
//! and every [`TICK`]. A stop sets the decision first, then waits for every
//! fetch and ignores their errors (closing the connection fails the others
//! as interrupted): the outcome comes from the decision.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};

use super::fetch_decision::Decision;
use super::fetch_one::{take_out, Received};
use super::not_ready::Waiting;
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{cancelled, DownloadError, DownloadEvent};
use crate::manifest::{Entry, FileStamp};
use crate::transport::{FetchStats, Fetcher, PathKind, PeerConnection, TransportError, GO};

/// How many small files' blobs are fetched at once: measured, ADR 0016.
pub(super) const FETCHING_AT_ONCE: usize = 32;

/// How often the round looks at the fetches when none lands.
pub(super) const TICK: Duration = Duration::from_millis(100);

/// How long a stop waits for the fetches to end. A fetch sees the stop only
/// when bytes come; one still waiting for its first is dropped after this.
pub(super) const STOP_WAIT: Duration = Duration::from_secs(3);

/// A file in the queue: fetching, or waiting on "not ready" (it keeps its
/// place, ADR 0016 rule 5).
#[derive(Debug)]
pub(super) struct Queued {
    /// The entry as it is saved here (its place in the folder).
    pub(super) entry: Entry,
    /// Its path in the stick, for its record.
    pub(super) stick_path: String,
    pub(super) replaceable: Option<FileStamp>,
    /// Waiting on "not ready", and when to ask again.
    waiting: Option<(Waiting, Instant)>,
}

impl Queued {
    pub(super) fn new(entry: Entry, stick_path: String, replaceable: Option<FileStamp>) -> Self {
        Self {
            entry,
            stick_path,
            replaceable,
            waiting: None,
        }
    }
}

/// A file whose blob is here and out of the store, for placing.
#[derive(Debug)]
pub(super) struct Landed {
    pub(super) queued: Queued,
    pub(super) received: Received,
}

/// What the round watches with, for all the fetches.
pub(super) struct Watch<'w, 'p> {
    pub(super) receiving: &'w mut ReceivingPath<'p>,
    pub(super) limiter: &'w mut Option<RelayLimiter>,
    pub(super) received_total: &'w mut u64,
}

type Fetching<'c> = BoxFuture<'c, ([u8; 32], Result<FetchStats, TransportError>)>;

pub(super) struct FetchQueue<'c, F: Fetcher, C: PeerConnection> {
    fetcher: &'c F,
    connection: &'c C,
    partial: &'c Path,
    in_flight: FuturesUnordered<Fetching<'c>>,
    files: HashMap<[u8; 32], Queued>,
    decision: Arc<Decision>,
}

impl<'c, F: Fetcher, C: PeerConnection> FetchQueue<'c, F, C> {
    pub(super) fn new(fetcher: &'c F, connection: &'c C, partial: &'c Path) -> Self {
        Self {
            fetcher,
            connection,
            partial,
            in_flight: FuturesUnordered::new(),
            files: HashMap::new(),
            decision: Arc::new(Decision::default()),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(super) fn has_room(&self) -> bool {
        self.files.len() < FETCHING_AT_ONCE
    }

    /// Whether a file with this blob is in the queue (a twin waits for it).
    pub(super) fn holds(&self, hash: &[u8; 32]) -> bool {
        self.files.contains_key(hash)
    }

    /// Starts fetching `queued`'s blob. Its place was checked; it is not in
    /// the queue already (twins wait, [`Self::holds`]).
    pub(super) fn push(&mut self, queued: Queued) {
        let hash = queued.entry.hash;
        self.files.insert(hash, queued);
        self.start(hash);
    }

    fn start(&mut self, hash: [u8; 32]) {
        let connection = self.connection;
        let decision = self.decision.clone();
        self.in_flight.push(Box::pin(async move {
            let mut previous = 0;
            let mut on_progress = |received: u64| {
                let new_bytes = received.saturating_sub(previous);
                previous = received;
                decision.bytes.fetch_add(new_bytes, Ordering::Relaxed);
                if decision.stop.load(Ordering::Relaxed) {
                    return ControlFlow::Break(());
                }
                if decision.sampler.due(new_bytes) && decision.limiter().is_some() {
                    decision.limit(connection.usage().relay);
                }
                let until = *decision.pause_until();
                match until {
                    Some(until) => {
                        ControlFlow::Continue(until.saturating_duration_since(Instant::now()))
                    }
                    None => GO,
                }
            };
            let fetched = connection.fetch_blob(&hash, &mut on_progress).await;
            (hash, fetched)
        }));
    }

    /// The next file whose blob is here, taken out of the store and told
    /// (`FileReceived`); `None` once the queue is empty. Watches the path,
    /// the relay limit and the user's stop meanwhile; on any way out but a
    /// landing every fetch has ended.
    pub(super) async fn next(
        &mut self,
        watch: &mut Watch<'_, '_>,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<Option<Landed>, DownloadError> {
        // The fetches run only while polled here: the limiter is theirs
        // meanwhile, and the round's again after.
        *self.decision.limiter() = watch.limiter.take();
        let next = self.next_landed(watch, on_event).await;
        *watch.limiter = self.decision.limiter().take();
        next
    }

    async fn next_landed(
        &mut self,
        watch: &mut Watch<'_, '_>,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<Option<Landed>, DownloadError> {
        loop {
            self.ask_again_due(Instant::now());
            if self.files.is_empty() {
                return Ok(None);
            }
            let landed = if self.in_flight.is_empty() {
                // Every file waits on "not ready": until the first is due.
                let due = self
                    .files
                    .values()
                    .filter_map(|queued| queued.waiting.as_ref().map(|(_, at)| *at))
                    .min()
                    .map_or(TICK, |at| at.saturating_duration_since(Instant::now()));
                tokio::time::sleep(due.min(TICK)).await;
                None
            } else {
                tokio::select! {
                    landed = self.in_flight.next() => landed,
                    () = tokio::time::sleep(TICK) => None,
                }
            };
            self.watch(watch, on_event).await?;
            let Some((hash, fetched)) = landed else {
                continue;
            };
            match fetched {
                Ok(stats) => return self.land(hash, stats, on_event).await.map(Some),
                Err(TransportError::NotReady) => self.not_ready(hash, on_event).await?,
                Err(error) => return Err(self.stop(cancelled(error)).await),
            }
        }
    }

    /// Stops every fetch and waits for them, ignoring how each ends; one
    /// that does not end within [`STOP_WAIT`] is dropped (what arrived of it
    /// stays in the batch's store).
    pub(super) async fn stop(&mut self, why: DownloadError) -> DownloadError {
        self.decision.stop.store(true, Ordering::Relaxed);
        let in_flight = &mut self.in_flight;
        let _ = tokio::time::timeout(STOP_WAIT, async {
            while in_flight.next().await.is_some() {}
        })
        .await;
        self.in_flight = FuturesUnordered::new();
        self.files.clear();
        why
    }

    fn ask_again_due(&mut self, now: Instant) {
        let due: Vec<[u8; 32]> = self
            .files
            .iter_mut()
            .filter(|(_, queued)| queued.waiting.as_ref().is_some_and(|(_, at)| *at <= now))
            .map(|(hash, queued)| {
                if let Some((_, at)) = queued.waiting.as_mut() {
                    // Not due again until it answers.
                    *at = now + Duration::from_secs(3600);
                }
                *hash
            })
            .collect();
        for hash in due {
            self.start(hash);
        }
    }

    async fn not_ready(
        &mut self,
        hash: [u8; 32],
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<(), DownloadError> {
        let now = Instant::now();
        let Some(queued) = self.files.get_mut(&hash) else {
            return Ok(());
        };
        let first = queued.waiting.is_none();
        let (waiting, _) = queued
            .waiting
            .get_or_insert_with(|| (Waiting::new(now), now));
        let pause = waiting.next_pause(now);
        if let (Some(pause), Some((_, at))) = (pause, queued.waiting.as_mut()) {
            *at = now + pause;
        }
        if first {
            let path = queued.entry.path.clone();
            if on_event(DownloadEvent::SenderPreparing { path: &path }).is_break() {
                return Err(self.stop(DownloadError::Cancelled).await);
            }
        }
        if pause.is_none() {
            return Err(self
                .stop(DownloadError::Transport(TransportError::NotReady))
                .await);
        }
        Ok(())
    }

    async fn land(
        &mut self,
        hash: [u8; 32],
        stats: FetchStats,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<Landed, DownloadError> {
        let Some(queued) = self.files.remove(&hash) else {
            return Err(self
                .stop(DownloadError::Transport(TransportError::Protocol(
                    "a fetch landed for no file".to_owned(),
                )))
                .await);
        };
        let (ciphertext, plaintext) =
            match take_out(self.fetcher, &queued.entry, self.partial).await {
                Ok(taken) => taken,
                Err(error) => return Err(self.stop(error).await),
            };
        let told = on_event(DownloadEvent::FileReceived {
            path: &queued.entry.path,
            size: queued.entry.size,
            already_local: stats.already_local,
            received: stats.received,
        });
        if told.is_break() {
            return Err(self.stop(DownloadError::Cancelled).await);
        }
        Ok(Landed {
            queued,
            received: Received {
                ciphertext,
                plaintext,
                already_local: stats.already_local,
                received: stats.received,
            },
        })
    }

    /// The round's look at all the fetches: the bytes that came, the path
    /// they came over, the relay limit, and a stop the user asked for.
    async fn watch(
        &mut self,
        watch: &mut Watch<'_, '_>,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<(), DownloadError> {
        let new_bytes = self.decision.bytes.swap(0, Ordering::Relaxed);
        *watch.received_total += new_bytes;
        let connection = self.connection;
        let mut told = ControlFlow::Continue(());
        if let Some(check) = watch
            .receiving
            .on_received(new_bytes, || connection.usage())
        {
            if check.look_again {
                // A stop the user asked for at its telling wins.
                if on_event(DownloadEvent::LookingForDirect).is_break() {
                    return Err(self.stop(DownloadError::Cancelled).await);
                }
                let why = self.stop(DownloadError::LookAgain).await;
                watch.receiving.looked();
                return Err(why);
            }
            if check.changed {
                told = on_event(DownloadEvent::PathChanged { path: check.kind });
            }
            let announced = {
                let mut limiter = self.decision.limiter();
                limiter
                    .as_mut()
                    .filter(|_| told.is_continue())
                    .and_then(|limiter| {
                        limiter
                            .announce(check.kind == PathKind::Relay)
                            .then(|| limiter.bits_per_second())
                    })
            };
            if let Some(bits_per_second) = announced {
                told = on_event(DownloadEvent::RelayLimited { bits_per_second });
            }
        }
        // ADR 0006: slowed to the limit, all fetches together, at every look
        // too (a pause runs out while no bytes come). `usage` is not free:
        // only with a limit.
        if self.decision.limiter().is_some() {
            self.decision.limit(connection.usage().relay);
        }
        if told.is_break() {
            return Err(self.stop(DownloadError::Cancelled).await);
        }
        Ok(())
    }
}

// Tested with two in-memory peers: `tests/several_at_once.rs`.
