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
    /// No direct path yet: waiting this long for one (ADR 0006). Before the
    /// start (`budget_used` false), or once the relay budget is used up.
    WaitingForDirect {
        seconds: u64,
        budget_used: bool,
    },
    /// This many bytes came through the relay, the whole budget; the download
    /// reconnects and looks for a direct path before going on.
    RelayBudgetUsed {
        bytes: u64,
    },
    /// After the budget was used up, the direct path was lost; the download
    /// reconnects and tries for a direct path again.
    DirectLost,
    Manifest {
        seq: u64,
        files: usize,
        bytes: u64,
        unchanged: bool,
    },
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
    #[error("no direct connection to the owner could be made and using the relay was declined ({remaining} bytes left); run again later, or allow the relay")]
    RelayDeclined { remaining: u64 },
    #[error("the direct connection to the owner kept dropping; run again to resume")]
    DirectLost,
    #[error("the free amount of data through the relay is used up; run again to resume")]
    RelayBudgetUsed,
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
