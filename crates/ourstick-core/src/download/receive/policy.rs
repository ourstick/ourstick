//! What a receive does after each error: wait and try again (saying why),
//! or stop (saying why). The one place that decides.

use std::time::Duration;

use super::super::{DownloadError, FOLLOW_EVERY};
use crate::space::SpaceError;
use crate::transport::TransportError;

/// Why a receive waits before trying again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// The owner cannot be reached (off, OurStick closed there, no network).
    Sender(String),
    /// The owner no longer shares the stick with this link (it may again).
    NoLongerShared,
    /// What comes does not fit on this computer.
    Room { needed: u64, available: u64 },
    /// The disk the stick is written to is full.
    DiskFull,
    /// The folder is not there, or not the stick's (an unplugged disk).
    Folder,
    /// The owner's folder keeps changing while it is downloaded.
    KeepsChanging,
    /// The owner's copy, or what came of it, is damaged; a new version of
    /// the stick may mend it.
    Damaged(String),
    /// This version cannot be taken here (its manifest cannot be read, or
    /// the folder's own partial folder is a symbolic link): the owner's next
    /// version, or the folder, may change. A symbolic link deeper in the
    /// folder is in the way like a file: the stick's file goes beside it. Names this computer refuses are not this:
    /// they are saved under a changed name (ADR 0010).
    CannotTake(String),
    /// Something on this computer failed (its network, its storage).
    ThisComputer(String),
}

/// What happens after a round of downloading failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Wait(Waiting, Duration),
    Stop,
}

/// The first pause after a failure the owner can fix by coming back.
pub(super) const FIRST_WAIT: Duration = FOLLOW_EVERY;
/// The longest pause between tries.
pub(super) const LONGEST_WAIT: Duration = Duration::from_secs(300);
/// How long a cancelled round may take to stop at its next event and finish
/// the file it is placing, before it is dropped.
pub(super) const GRACE: Duration = Duration::from_secs(10);

/// What to do after `error`, the round's room refusal being `refused` and
/// `waited` the last pause for the same reason: the one place that decides.
/// Nothing is given up on that the owner or this computer can mend; only a
/// record that must not be taken, the store and the programs' own refusals
/// stop, saying why.
pub fn after_error(error: &DownloadError, refused: Option<&SpaceError>, waited: Duration) -> Next {
    use std::io::ErrorKind;
    let longer = (waited * 2).clamp(FIRST_WAIT, LONGEST_WAIT);
    let said = || crate::untrusted::escape(&error.to_string());
    let wait = |why| Next::Wait(why, longer);
    match (error, refused) {
        (_, Some(SpaceError::DoesNotFit { needed, available })) => Next::Wait(
            Waiting::Room {
                needed: *needed,
                available: *available,
            },
            LONGEST_WAIT,
        ),
        // The ledger could not be read or written: said, and tried again.
        (_, Some(other)) => wait(Waiting::ThisComputer(crate::untrusted::escape(
            &other.to_string(),
        ))),
        (DownloadError::Transport(transport), _) => match transport {
            TransportError::UnknownStick => Next::Wait(Waiting::NoLongerShared, LONGEST_WAIT),
            TransportError::Unreachable(reason) | TransportError::Interrupted(reason) => {
                wait(Waiting::Sender(crate::untrusted::escape(reason)))
            }
            TransportError::NoRecord | TransportError::NotReady | TransportError::Protocol(_) => {
                wait(Waiting::Sender(said()))
            }
            TransportError::Refused => wait(Waiting::KeepsChanging),
            TransportError::OwnerFailed | TransportError::Corrupt => {
                Next::Wait(Waiting::Damaged(said()), LONGEST_WAIT)
            }
            TransportError::Bind(_) | TransportError::Storage(_) | TransportError::Io(_) => {
                wait(Waiting::ThisComputer(said()))
            }
            TransportError::Cancelled => Next::Stop,
        },
        (DownloadError::Io { source, .. }, _) => match source.kind() {
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => {
                Next::Wait(Waiting::DiskFull, LONGEST_WAIT)
            }
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                Next::Wait(Waiting::Folder, FIRST_WAIT)
            }
            _ => wait(Waiting::ThisComputer(said())),
        },
        (DownloadError::UnsafePath { .. } | DownloadError::Manifest(_), _) => {
            Next::Wait(Waiting::CannotTake(said()), LONGEST_WAIT)
        }
        (DownloadError::SizeMismatch { .. } | DownloadError::Cipher(_), _) => {
            Next::Wait(Waiting::Damaged(said()), LONGEST_WAIT)
        }
        // A record that must not be taken (older than this device has seen,
        // a wrong signature), the store, a cancel from outside, and the
        // programs' own refusals: they need the user.
        _ => Next::Stop,
    }
}
