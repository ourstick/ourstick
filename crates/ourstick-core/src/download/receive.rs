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

use super::{download_as, wait_for_newer, DownloadError, DownloadEvent, RelayPolicy};
use crate::blocking::run_blocking;
use crate::clock::Clock;
use crate::space::DownloadRoom;
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

/// Whether `folder` is there and marked as `stick_id`'s (reading no more
/// of the mark than an id takes). Blocking.
pub fn received_folder_there(folder: &Path, stick_id: &StickId) -> bool {
    use std::io::Read;
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    let mut mark = Vec::new();
    std::fs::File::open(folder.join(RECEIVED_MARKER))
        .and_then(|file| file.take(hex.len() as u64 + 1).read_to_end(&mut mark))
        .is_ok()
        && mark == hex.as_bytes()
}

/// Records the stick as received by `kind` into `folder` before its first
/// round (the receiver window adds it): the same one place a download writes
/// the row in. The caller holds the stick's download lock.
pub async fn take_stick(
    store: &Arc<dyn Store>,
    stick_id: StickId,
    folder: &Path,
    kind: StickKind,
    now_ms: u64,
) -> Result<(), DownloadError> {
    super::joined::prepare(store, stick_id, folder, kind, now_ms).await
}

/// The partial folder of a round into `output_dir`; for the window's stick
/// (`marked`), only while the folder still bears its mark, checked right
/// before: a disk unplugged since the round began leaves an empty mount
/// point, which is never filled.
pub(super) fn partial_for(
    output_dir: &Path,
    marked: Option<StickId>,
) -> Result<std::path::PathBuf, DownloadError> {
    if let Some(stick_id) = marked {
        if !received_folder_there(output_dir, &stick_id) {
            return Err(DownloadError::Io {
                path: output_dir.to_path_buf(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
    }
    super::files::prepare_partial_dir(output_dir)
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

/// What a receive tells, besides each download's own events.
#[derive(Debug)]
pub enum ReceiveEvent<'a> {
    Download(DownloadEvent<'a>),
    /// Every file of version `seq` is here; waiting for the next one. Told
    /// again when the owner can be reached again after `Waiting`.
    UpToDate {
        seq: u64,
    },
    /// Trying again in `after`, because of `why`. While up to date, told
    /// when the owner cannot be reached (asked again every `after`).
    Waiting {
        why: Waiting,
        after: Duration,
    },
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
    /// How often the owner is asked for a newer version (`FOLLOW_EVERY`).
    pub follow_every: Duration,
}

/// Gives the room back however a receive ends.
struct GiveBack<'a>(&'a DownloadRoom);

impl Drop for GiveBack<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Receives the stick into its marked folder and keeps it up to date until
/// `cancel` (then `Ok`), or until an error that needs the user (`Err`,
/// saying why). The caller holds the stick's download lock and shuts the
/// fetcher down after this returns (keeping its store, so the next receive
/// resumes). A cancelled round stops at its next event, finishing the file
/// it is placing (at most [`GRACE`]); a file being placed past that may
/// still land a moment after this returns, and is found at the next round.
pub async fn receive<F: Fetcher>(
    receiving: &Receiving<'_, F>,
    cancel: &Cancel,
    on_event: &mut (dyn FnMut(ReceiveEvent<'_>) + Send),
) -> Result<(), DownloadError> {
    let _give_back = GiveBack(receiving.room);
    let stick_id = receiving.ticket.stick_id;
    let mut waited = Duration::ZERO;
    let mut last_why: Option<std::mem::Discriminant<Waiting>> = None;
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let folder = receiving.folder.to_path_buf();
        if !run_blocking(move || received_folder_there(&folder, &stick_id)).await {
            // Told once, then looked for quietly every FIRST_WAIT.
            let folder_why = Some(std::mem::discriminant(&Waiting::Folder));
            if last_why != folder_why {
                last_why = folder_why;
                on_event(ReceiveEvent::Waiting {
                    why: Waiting::Folder,
                    after: FIRST_WAIT,
                });
            }
            if !pause(FIRST_WAIT, cancel).await {
                return Ok(());
            }
            continue;
        }
        let round = {
            let round = one_round(receiving, cancel, &mut *on_event);
            tokio::pin!(round);
            tokio::select! {
                round = &mut round => Some(round),
                () = cancel.cancelled() => {
                    let _ = tokio::time::timeout(GRACE, &mut round).await;
                    None
                }
            }
        };
        // Cancelled while the round ended (it may have stopped *because*
        // of the cancel): a stop the user asked for, not an error.
        let Some(round) = round.filter(|_| !cancel.is_cancelled()) else {
            return Ok(());
        };
        receiving.room.release();
        let error = match round {
            Ok(seq) => {
                waited = Duration::ZERO;
                last_why = None;
                on_event(ReceiveEvent::UpToDate { seq });
                match follow(receiving, cancel, seq, &mut *on_event).await {
                    None => return Ok(()),
                    Some(Ok(())) => continue,
                    Some(Err(error)) => error,
                }
            }
            Err(error) => error,
        };
        if cancel.is_cancelled() {
            return Ok(());
        }
        let refused = receiving.room.refusal();
        let mut next = after_error(&error, refused.as_ref(), waited);
        // A new reason starts from the first pause again.
        if let Next::Wait(why, _) = &next {
            let why = std::mem::discriminant(why);
            if last_why != Some(why) {
                last_why = Some(why);
                next = after_error(&error, refused.as_ref(), Duration::ZERO);
            }
        }
        match next {
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

/// While up to date: waits for the owner's next version, telling when the
/// owner cannot be reached (or no longer shares the stick) and when it can
/// again. `None` if cancelled.
async fn follow<F: Fetcher>(
    receiving: &Receiving<'_, F>,
    cancel: &Cancel,
    seq: u64,
    on_event: &mut (dyn FnMut(ReceiveEvent<'_>) + Send),
) -> Option<Result<(), DownloadError>> {
    let after = receiving.follow_every;
    let mut told = |event: super::FollowEvent<'_>| match event {
        super::FollowEvent::Unreachable { reason } => {
            let why = match reason {
                TransportError::UnknownStick => Waiting::NoLongerShared,
                other => Waiting::Sender(crate::untrusted::escape(&other.to_string())),
            };
            on_event(ReceiveEvent::Waiting { why, after });
        }
        super::FollowEvent::Reachable => on_event(ReceiveEvent::UpToDate { seq }),
    };
    tokio::select! {
        newer = wait_for_newer(receiving.fetcher, receiving.ticket, seq, after, &mut told) => {
            Some(newer.map(|_| ()))
        }
        () = cancel.cancelled() => None,
    }
}

/// One download, its events passed on and to the room, stopping at its next
/// event once cancelled; the version it completed.
async fn one_round<F: Fetcher>(
    receiving: &Receiving<'_, F>,
    cancel: &Cancel,
    on_event: &mut (dyn FnMut(ReceiveEvent<'_>) + Send),
) -> Result<u64, DownloadError> {
    let room = receiving.room;
    let mut forward = |event: DownloadEvent<'_>| {
        if cancel.is_cancelled() {
            return std::ops::ControlFlow::Break(());
        }
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

mod policy;

#[cfg(test)]
use policy::LONGEST_WAIT;
pub use policy::{after_error, Next, Waiting};
use policy::{FIRST_WAIT, GRACE};

#[cfg(test)]
mod tests;
