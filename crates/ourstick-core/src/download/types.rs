//! What a download reports: events while it runs, a report when it is done,
//! and why it failed.

use std::path::PathBuf;

use crate::cipher::CipherError;
use crate::manifest::ManifestError;
use crate::record::RecordError;
use crate::store::StoreError;
use crate::transport::{PathKind, PathUsage, TransportError};

/// Progress of a download, for display. Return [`ControlFlow::Break`] from the
/// callback to cancel.
#[derive(Debug, Clone, Copy)]
pub enum DownloadEvent<'a> {
    Connected {
        path: PathKind,
    },
    /// The data now mostly arrives over another kind of path.
    PathChanged {
        path: PathKind,
    },
    /// Data now arrives through a relay and is limited to this rate
    /// (ADR 0006). Lasts until a `PathChanged` to a direct path.
    RelayLimited {
        bits_per_second: u64,
    },
    /// The owner published a new version while this one downloaded; the
    /// download goes on with it (files already done are not fetched again).
    NewVersion {
        seq: u64,
    },
    Manifest {
        seq: u64,
        files: usize,
        bytes: u64,
        unchanged: bool,
        /// The owner is still reading its folder (ADR 0007): more files come
        /// in later versions, and the download waits for them.
        partial: bool,
    },
    /// Every file of a partial version is here; waiting for the owner's next
    /// version.
    WaitingForMore {
        seq: u64,
    },
    /// While waiting for more, the owner cannot be reached; still trying.
    /// Reported once per outage.
    OwnerUnreachable {
        reason: &'a TransportError,
    },
    /// The owner can be reached again.
    OwnerReachable,
    FileStarted {
        path: &'a str,
        size: u64,
        /// Blob bytes already stored locally from an earlier, interrupted run.
        already_local: u64,
    },
    Progress {
        path: &'a str,
        received: u64,
    },
    FileDone {
        path: &'a str,
        already_local: u64,
        received: u64,
    },
    /// The folder already holds this file with this content: not fetched.
    FileAlreadyHere {
        path: &'a str,
        size: u64,
    },
    /// The folder holds something else at this path, changed there since
    /// `get` wrote it or not written by `get`: left as it is.
    FileChangedHere {
        path: &'a str,
        size: u64,
    },
    /// Files `get` wrote earlier that are no longer in the stick; kept.
    NoLongerInStick {
        count: usize,
    },
    /// Another file of this download has the same content: copied from it,
    /// not fetched again.
    FileCopiedHere {
        path: &'a str,
        size: u64,
    },
    /// The owner is still preparing this file (ADR 0007); the download waits
    /// and asks again. Once per file.
    SenderPreparing {
        path: &'a str,
    },
}

/// What a finished download did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadReport {
    pub seq: u64,
    pub files: usize,
    pub bytes: u64,
    /// Blob bytes received over the network (less than the total after a resume).
    pub received: u64,
    /// Files the folder already held, not fetched.
    pub already_here: usize,
    /// Files changed in the folder and left as they were.
    pub changed_here: usize,
    /// Bytes that arrived over direct and over relay paths, over all
    /// connections of this download.
    pub usage: PathUsage,
}

/// Why a download failed. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("the download was cancelled; run the same command again to resume")]
    Cancelled,
    #[error("the owner's manifest record was rejected: {0}")]
    Record(#[from] RecordError),
    #[error("{path}: expected {expected} bytes, got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("refusing to write {path}: {reason}")]
    UnsafePath { path: String, reason: &'static str },
    #[error("{first} and {second} would be the same file on a disk that ignores letter case, or one is a file and the other needs it as a folder")]
    PathCollision { first: String, second: String },
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Cipher(#[from] CipherError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
