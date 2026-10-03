//! A received stick's reports to its owner (ADR 0015, rule 1): what a
//! receive's events say, sent when it changes, about every 30 seconds while
//! files come, and again when what could not go can.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;

use super::receive::Waiting;
use super::{DownloadEvent, ReceiveEvent};
use crate::blocking::run_blocking;
use crate::clock::Clock;
use crate::report::{
    seal, ComputerKind, ReaderKey, ReaderReport, ReportError, ReportState, WaitReason,
};
use crate::store::{Store, StoreError};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, TransportError};

/// Where reports go: every [`Fetcher`], and a test's owner.
pub trait ReportTo: Send + Sync {
    fn report(
        &self,
        ticket: &Ticket,
        sealed: &[u8],
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;
}

impl<F: Fetcher> ReportTo for F {
    fn report(
        &self,
        ticket: &Ticket,
        sealed: &[u8],
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send {
        Fetcher::report(self, ticket, sealed)
    }
}

/// How often a report goes while files come, and how soon one that could
/// not go is tried again.
pub const REPORT_EVERY: Duration = Duration::from_secs(30);

/// How long one report may take to reach the owner.
pub const REPORT_DEADLINE: Duration = Duration::from_secs(5);

/// Why a report did not reach the owner. Never in the way of a download.
#[derive(Debug, thiserror::Error)]
pub enum NotSent {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the owner did not take the report in time")]
    TimedOut,
}

/// What the latest events say, and whether the owner has it.
#[derive(Debug, Default)]
struct Latest {
    seq: Option<u64>,
    state: Option<ReportState>,
    /// Grows with every change, so a report sent while another change came
    /// does not count as sending that one.
    version: u64,
    sent_version: u64,
    /// The change is more than a percent: it goes at once.
    urgent: bool,
    last_try: Option<Instant>,
}

impl Latest {
    fn note(&mut self, seq: Option<u64>, state: ReportState, urgent: bool) {
        if seq.is_some() {
            self.seq = seq;
        }
        self.state = Some(state);
        self.version += 1;
        self.urgent |= urgent;
    }

    /// What should go now, if anything: an unsent state of a known version,
    /// at once when urgent, else [`REPORT_EVERY`] after the last try. A
    /// sender away is not told it is away.
    fn due(&self, now: Instant, every: Duration) -> Option<(u64, u64, ReportState)> {
        let (seq, state) = (self.seq?, self.state?);
        if self.sent_version == self.version
            || state == ReportState::Waiting(WaitReason::SenderAway)
        {
            return None;
        }
        let waited = self
            .last_try
            .is_none_or(|last| now.duration_since(last) >= every);
        (self.urgent || waited).then_some((self.version, seq, state))
    }
}

/// One received stick's reporter. Holds the ticket (the stick key): never
/// printed.
pub struct Reporter {
    ticket: Ticket,
    reader_key: ReaderKey,
    kind: ComputerKind,
    name: Option<String>,
    store: Arc<dyn Store>,
    clock: Arc<dyn Clock>,
    latest: Mutex<Latest>,
    wake: tokio::sync::Notify,
    every: Duration,
}

impl Reporter {
    /// `name` is only one the receiver typed (never the computer's).
    pub fn new(
        ticket: Ticket,
        reader_key: ReaderKey,
        name: Option<String>,
        store: Arc<dyn Store>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            ticket,
            reader_key,
            kind: ComputerKind::this(),
            name,
            store,
            clock,
            latest: Mutex::new(Latest::default()),
            wake: tokio::sync::Notify::new(),
            every: REPORT_EVERY,
        }
    }

    /// Reports every `every` instead of [`REPORT_EVERY`] (tests).
    #[cfg(test)]
    fn every(self, every: Duration) -> Self {
        Self { every, ..self }
    }

    fn latest(&self) -> MutexGuard<'_, Latest> {
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes a receive's event; `done` of `total` are the bytes of this
    /// round here and planned (what the window's bar shows).
    pub fn on_event(&self, event: &ReceiveEvent<'_>, done: u64, total: u64) {
        // The version a round works on; nothing to tell yet.
        if let ReceiveEvent::Download(
            DownloadEvent::Manifest { seq, .. } | DownloadEvent::NewVersion { seq },
        ) = event
        {
            let mut latest = self.latest();
            latest.seq = Some(*seq);
            // A change waiting for its version can go now.
            if latest.urgent {
                self.wake.notify_one();
            }
            return;
        }
        let Some((seq, state, urgent)) = noted(event, done, total) else {
            return;
        };
        self.latest().note(seq, state, urgent);
        if urgent {
            self.wake.notify_one();
        }
    }

    /// Sends what is due, for as long as the receive runs: the caller
    /// selects it beside the receive, so it never ends on its own.
    pub async fn pump(&self, fetcher: &impl ReportTo) {
        loop {
            let due = {
                let mut latest = self.latest();
                let now = Instant::now();
                let due = latest.due(now, self.every);
                if due.is_some() {
                    // Taken now: a change while it goes stays urgent.
                    latest.urgent = false;
                    latest.last_try = Some(now);
                }
                due
            };
            if let Some((version, seq, state)) = due {
                let sent = self.send(fetcher, seq, state).await;
                let mut latest = self.latest();
                latest.last_try = Some(Instant::now());
                match sent {
                    Ok(()) => latest.sent_version = latest.sent_version.max(version),
                    Err(error) => tracing::debug!(%error, "a report to the owner did not go"),
                }
                continue;
            }
            tokio::select! {
                () = self.wake.notified() => {}
                () = tokio::time::sleep(self.every) => {}
            }
        }
    }

    /// The version reports are about until a round says another (a
    /// reporter made only to tell a pause or a forget).
    pub fn set_version(&self, seq: u64) {
        self.latest().seq = Some(seq);
    }

    /// Sends `state` now, about the latest version known (pausing, or
    /// forgetting the stick), within `deadline`.
    pub async fn send_now(
        &self,
        fetcher: &impl ReportTo,
        state: ReportState,
        deadline: Duration,
    ) -> Result<(), NotSent> {
        let seq = self.latest().seq.unwrap_or(0);
        tokio::time::timeout(deadline, self.send(fetcher, seq, state))
            .await
            .map_err(|_| NotSent::TimedOut)?
    }

    async fn send(
        &self,
        fetcher: &impl ReportTo,
        seq: u64,
        state: ReportState,
    ) -> Result<(), NotSent> {
        let store = self.store.clone();
        let stick_id = self.ticket.stick_id;
        let now_ms = self.clock.now_ms();
        let (counter, everything) = run_blocking(move || {
            Ok::<_, StoreError>((
                store.next_report_counter(&stick_id, now_ms)?,
                store.left_out(&stick_id)?.is_empty(),
            ))
        })
        .await?;
        let report = ReaderReport {
            counter,
            seq,
            state,
            everything,
            kind: self.kind,
            name: self.name.clone(),
        };
        let sealed = seal(
            &report,
            &self.reader_key,
            &self.ticket.stick_id,
            &self.ticket.stick_key,
        )?;
        tokio::time::timeout(
            REPORT_DEADLINE,
            ReportTo::report(fetcher, &self.ticket, &sealed),
        )
        .await
        .map_err(|_| NotSent::TimedOut)??;
        Ok(())
    }
}

/// What an event says for a report: the version it names, the state, and
/// whether it goes at once.
fn noted(
    event: &ReceiveEvent<'_>,
    done: u64,
    total: u64,
) -> Option<(Option<u64>, ReportState, bool)> {
    let downloading = |urgent| {
        let percent = if total == 0 {
            0
        } else {
            let percent = u128::from(done.min(total)) * 100 / u128::from(total);
            u8::try_from(percent).unwrap_or(99).min(99)
        };
        Some((None, ReportState::Downloading { percent }, urgent))
    };
    match event {
        ReceiveEvent::Download(DownloadEvent::Planned { .. }) => downloading(true),
        ReceiveEvent::Download(
            DownloadEvent::FileStarted { .. }
            | DownloadEvent::Progress { .. }
            | DownloadEvent::FileReceived { .. }
            | DownloadEvent::FileCopiedHere { .. },
        ) => downloading(false),
        // Everything so far, while the sender still reads its folder.
        ReceiveEvent::Download(DownloadEvent::WaitingForMore { seq }) => {
            Some((Some(*seq), ReportState::UpToDate, true))
        }
        ReceiveEvent::UpToDate { seq } => Some((Some(*seq), ReportState::UpToDate, true)),
        ReceiveEvent::Waiting { why, .. } => {
            Some((None, ReportState::Waiting(wait_reason(why)), true))
        }
        ReceiveEvent::Download(_) => None,
    }
}

/// The reason a report gives for a wait: one of a few, never words.
pub fn wait_reason(why: &Waiting) -> WaitReason {
    match why {
        Waiting::Sender(_) => WaitReason::SenderAway,
        Waiting::Room { .. } | Waiting::DiskFull => WaitReason::NoRoom,
        Waiting::Folder => WaitReason::FolderMissing,
        _ => WaitReason::Other,
    }
}

#[cfg(test)]
#[path = "reporter_tests.rs"]
mod tests;
