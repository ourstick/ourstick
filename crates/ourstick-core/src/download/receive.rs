//! Receiving a stick and keeping it up to date (the receiver window, step
//! 7b): download, wait for the owner's next version, download again, until
//! cancelled. What happens on each error is decided here, in one place: wait
//! and try again (and say why), or stop (and say why). The folder must be
//! the one marked for the stick: an unplugged disk is never filled in its
//! place.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use super::{download_as, wait_for_newer, DownloadError, DownloadEvent, RelayPolicy, FOLLOW_EVERY};
use crate::clock::Clock;
use crate::space::{DownloadRoom, SpaceError};
use crate::stick_id::StickId;
use crate::store::{StickKind, Store};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, TransportError};

/// The mark of a received stick's folder: the stick's id. Written when the
/// folder is chosen; a folder without it (an unplugged disk's empty mount
/// point, a deleted folder) is not available.
pub const RECEIVED_MARKER: &str = ".ourstick-received";

/// Marks `folder` as the received stick `stick_id`'s.
pub fn mark_received_folder(folder: &Path, stick_id: &StickId) -> std::io::Result<()> {
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    std::fs::write(folder.join(RECEIVED_MARKER), hex)
}

/// Whether `folder` is there and marked as `stick_id`'s.
pub fn received_folder_there(folder: &Path, stick_id: &StickId) -> bool {
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    std::fs::read(folder.join(RECEIVED_MARKER)).is_ok_and(|mark| mark == hex.as_bytes())
}

/// Tells a receive to stop: whatever it is doing (downloading, waiting for
/// the owner, waiting before trying again) ends at once.
#[derive(Debug, Default)]
pub struct Cancel {
    cancelled: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Returns once cancelled.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// Why a receive waits before trying again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// The owner cannot be reached (off, OurStick closed there, no network).
    Sender(String),
    /// The owner no longer shares the stick with this link (it may again).
    NoLongerShared,
    /// What comes does not fit on this computer.
    Room { needed: u64, available: u64 },
    /// The folder is not there, or not the stick's (an unplugged disk).
    Folder,
    /// The owner's folder keeps changing while it is downloaded.
    KeepsChanging,
}

/// What a receive tells, besides each download's own events.
#[derive(Debug)]
pub enum ReceiveEvent<'a> {
    Download(DownloadEvent<'a>),
    /// Every file of version `seq` is here; waiting for the next one.
    UpToDate {
        seq: u64,
    },
    /// Trying again in `after`, because of `why`.
    Waiting {
        why: Waiting,
        after: Duration,
    },
}

/// What happens after a round of downloading failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Wait(Waiting, Duration),
    Stop,
}

/// The first pause after a failure the owner can fix by coming back.
const FIRST_WAIT: Duration = FOLLOW_EVERY;
/// The longest pause between tries.
const LONGEST_WAIT: Duration = Duration::from_secs(300);

/// What to do after `error`, the round's room refusal being `refused` and
/// `waited` the last pause for the same reason: the one place that decides.
pub fn after_error(error: &DownloadError, refused: Option<&SpaceError>, waited: Duration) -> Next {
    let longer = (waited * 2).clamp(FIRST_WAIT, LONGEST_WAIT);
    match (error, refused) {
        (_, Some(SpaceError::DoesNotFit { needed, available })) => Next::Wait(
            Waiting::Room {
                needed: *needed,
                available: *available,
            },
            LONGEST_WAIT,
        ),
        (DownloadError::Transport(TransportError::UnknownStick), _) => {
            Next::Wait(Waiting::NoLongerShared, LONGEST_WAIT)
        }
        (
            DownloadError::Transport(
                TransportError::Unreachable(reason) | TransportError::Interrupted(reason),
            ),
            _,
        ) => Next::Wait(Waiting::Sender(reason.clone()), longer),
        (
            DownloadError::Transport(
                TransportError::NoRecord | TransportError::Bind(_) | TransportError::Io(_),
            ),
            _,
        ) => Next::Wait(Waiting::Sender(error.to_string()), longer),
        (DownloadError::Transport(TransportError::Refused | TransportError::OwnerFailed), _) => {
            Next::Wait(Waiting::KeepsChanging, longer)
        }
        (DownloadError::Io { source, .. }, _) => match source.kind() {
            std::io::ErrorKind::StorageFull => Next::Wait(
                Waiting::Room {
                    needed: 0,
                    available: 0,
                },
                LONGEST_WAIT,
            ),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                Next::Wait(Waiting::Folder, FIRST_WAIT)
            }
            _ => Next::Stop,
        },
        // A bad record (an older version than this device has seen, a
        // wrong signature), unsafe paths, damaged data, the store, and the
        // app's or `get`'s own refusals: they need the user.
        _ => Next::Stop,
    }
}

/// What a receive works with.
pub struct Receiving<'a, F: Fetcher> {
    pub kind: StickKind,
    pub fetcher: &'a F,
    pub store: Arc<dyn Store>,
    pub clock: &'a dyn Clock,
    pub ticket: &'a Ticket,
    pub folder: &'a Path,
    pub policy: &'a RelayPolicy,
    pub room: &'a DownloadRoom,
    /// How often the owner is asked for a newer version ([`FOLLOW_EVERY`]).
    pub follow_every: Duration,
}

/// Receives the stick into its marked folder and keeps it up to date until
/// `cancel` (then `Ok`), or until an error that needs the user (`Err`,
/// saying why). The caller holds the stick's download lock and shuts the
/// fetcher down after this returns (keeping its store, so the next receive
/// resumes).
pub async fn receive<F: Fetcher>(
    receiving: &Receiving<'_, F>,
    cancel: &Cancel,
    on_event: &mut (dyn FnMut(ReceiveEvent<'_>) + Send),
) -> Result<(), DownloadError> {
    let stick_id = receiving.ticket.stick_id;
    let mut waited = Duration::ZERO;
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if !received_folder_there(receiving.folder, &stick_id) {
            waited = FIRST_WAIT;
            on_event(ReceiveEvent::Waiting {
                why: Waiting::Folder,
                after: waited,
            });
            if !pause(waited, cancel).await {
                return Ok(());
            }
            continue;
        }
        let round = tokio::select! {
            round = one_round(receiving, on_event) => round,
            () = cancel.cancelled() => return Ok(()),
        };
        receiving.room.release();
        let error = match round {
            Ok(seq) => {
                waited = Duration::ZERO;
                on_event(ReceiveEvent::UpToDate { seq });
                let mut quiet = |_: super::FollowEvent<'_>| {};
                let newer = tokio::select! {
                    newer = wait_for_newer(
                        receiving.fetcher,
                        receiving.ticket,
                        seq,
                        receiving.follow_every,
                        &mut quiet,
                    ) => newer,
                    () = cancel.cancelled() => return Ok(()),
                };
                match newer {
                    Ok(_) => continue,
                    Err(error) => error,
                }
            }
            Err(error) => error,
        };
        let refused = receiving.room.refusal();
        match after_error(&error, refused.as_ref(), waited) {
            Next::Stop => return Err(error),
            Next::Wait(why, after) => {
                waited = after;
                on_event(ReceiveEvent::Waiting { why, after });
                if !pause(after, cancel).await {
                    return Ok(());
                }
            }
        }
    }
}

/// One download, its events passed on and to the room; the version it
/// completed.
async fn one_round<F: Fetcher>(
    receiving: &Receiving<'_, F>,
    on_event: &mut (dyn FnMut(ReceiveEvent<'_>) + Send),
) -> Result<u64, DownloadError> {
    let room = receiving.room;
    let mut forward = |event: DownloadEvent<'_>| {
        let flow = room.on_event(&event);
        on_event(ReceiveEvent::Download(event));
        flow
    };
    let report = download_as(
        receiving.kind,
        receiving.fetcher,
        receiving.store.clone(),
        receiving.clock,
        receiving.ticket,
        receiving.folder,
        receiving.policy,
        &mut forward,
    )
    .await?;
    Ok(report.seq)
}

/// Waits `after`; `false` if cancelled meanwhile.
async fn pause(after: Duration, cancel: &Cancel) -> bool {
    tokio::select! {
        () = tokio::time::sleep(after) => true,
        () = cancel.cancelled() => false,
    }
}

#[cfg(test)]
mod tests;
