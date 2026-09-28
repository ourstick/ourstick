//! Moving records and blobs between peers.
//!
//! The owner side is a [`Publisher`], the reader side a [`Fetcher`] that opens
//! [`PeerConnection`]s. Nothing outside this module imports iroh or iroh-blobs:
//! the implementation in [`iroh`] can be replaced without touching the rest of
//! the core (ADR 0005).

pub mod iroh;

use std::collections::HashSet;
use std::future::Future;
use std::ops::ControlFlow;
use std::path::Path;

use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId, Ticket};

/// A BLAKE3 hash of a blob.
pub type BlobHash = [u8; 32];

/// How a connection currently reaches the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// Directly, over UDP (after hole punching, or on the same network).
    Direct,
    /// Through a relay server.
    Relay,
    /// Not known yet.
    Unknown,
}

impl std::fmt::Display for PathKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Direct => "direct",
            Self::Relay => "relay",
            Self::Unknown => "unknown",
        })
    }
}

/// Bytes received from the peer over each kind of path during a connection
/// (UDP datagrams, so including a little protocol overhead).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathUsage {
    pub direct: u64,
    pub relay: u64,
}

/// Counters for one blob fetch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchStats {
    /// Blob bytes that were already local before the fetch.
    pub already_local: u64,
    /// Blob bytes received over the network in this fetch.
    pub received: u64,
}

/// Why a transfer failed. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("could not start networking: {0}")]
    Bind(String),
    #[error("could not reach the owner of this stick: {0}")]
    Unreachable(String),
    #[error("the owner has no manifest to offer for this stick yet")]
    NoRecord,
    #[error("the owner does not share this stick (anymore)")]
    UnknownStick,
    #[error("the owner refused to send this data")]
    Refused,
    #[error("received data does not match its hash; the owner's copy is damaged")]
    Corrupt,
    #[error("the owner could not read its own copy of the data (it may be damaged); ask the owner to share again")]
    OwnerFailed,
    #[error("the transfer was interrupted: {0}")]
    Interrupted(String),
    #[error("the transfer was cancelled")]
    Cancelled,
    #[error("the peer sent an invalid response: {0}")]
    Protocol(String),
    #[error("local blob storage failed: {0}")]
    Storage(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The owner's side: holds the encrypted blobs and serves the current record.
pub trait Publisher: Send + Sync {
    /// This device's node id, for tickets.
    fn node_id(&self) -> NodeId;

    /// Where this node can be reached now, for tickets.
    fn address_hints(&self) -> Vec<AddressHint>;

    /// Where the encrypted blob cache lives on disk.
    fn cache_dir(&self) -> &Path;

    /// A directory on the same file system as the cache, for blobs that are
    /// being prepared.
    fn staging_dir(&self) -> &Path;

    /// Imports an encrypted blob file into the cache and returns its hash. The
    /// blob is kept at least until the next [`Publisher::publish`] that does
    /// not include it. It is not served until a publish includes it.
    fn import_blob(
        &self,
        blob_file: &Path,
    ) -> impl Future<Output = Result<BlobHash, TransportError>> + Send;

    /// Starts serving `record`, and only the blobs in `allowed`. Blobs not in
    /// `allowed` stop being served at once and are removed from the cache later.
    fn publish(
        &self,
        record: SignedManifestRecord,
        allowed: HashSet<BlobHash>,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Whether the cache currently holds the complete blob.
    fn has_blob(
        &self,
        hash: &BlobHash,
    ) -> impl Future<Output = Result<bool, TransportError>> + Send;

    /// Stops serving and closes the cache cleanly.
    fn shutdown(self) -> impl Future<Output = ()> + Send;
}

/// The reader's side: a local blob store and connections to owners.
pub trait Fetcher: Send + Sync {
    type Connection: PeerConnection;

    /// Connects to the owner named in `ticket`.
    fn connect(
        &self,
        ticket: &Ticket,
    ) -> impl Future<Output = Result<Self::Connection, TransportError>> + Send;

    /// A complete local blob, read into memory. For small blobs (the manifest).
    fn read_blob(
        &self,
        hash: &BlobHash,
    ) -> impl Future<Output = Result<Vec<u8>, TransportError>> + Send;

    /// Writes a complete local blob to `target` (a new file).
    fn export_blob(
        &self,
        hash: &BlobHash,
        target: &Path,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// How many bytes of the blob are already stored locally.
    fn local_bytes(
        &self,
        hash: &BlobHash,
    ) -> impl Future<Output = Result<u64, TransportError>> + Send;

    /// Closes the local store cleanly, keeping partial blobs for a later resume
    /// (`keep = true`) or deleting the whole store (`keep = false`).
    fn shutdown(self, keep: bool) -> impl Future<Output = Result<(), TransportError>> + Send;
}

/// A connection from a reader to an owner.
pub trait PeerConnection: Send + Sync {
    /// The path this side currently *sends* on. With multipath QUIC the two
    /// sides choose independently: a reader that sends little can stay on the
    /// relay while the data comes over a direct path. To know how the data
    /// arrives, compare [`PeerConnection::usage`] over time.
    fn path(&self) -> PathKind;

    /// Receives the path every time it changes, until the connection closes.
    fn path_changes(&self) -> tokio::sync::watch::Receiver<PathKind>;

    /// How many bytes arrived over direct and over relay paths so far.
    fn usage(&self) -> PathUsage;

    /// Asks the owner for the latest signed record of `stick_id`. The record is
    /// not verified here.
    fn fetch_record(
        &self,
        stick_id: &StickId,
    ) -> impl Future<Output = Result<SignedManifestRecord, TransportError>> + Send;

    /// Downloads the missing parts of a blob into the local store, verifying
    /// every chunk against `hash`. `on_progress` gets the bytes received so far
    /// in this fetch and can cancel it.
    fn fetch_blob(
        &self,
        hash: &BlobHash,
        on_progress: &mut (dyn FnMut(u64) -> ControlFlow<()> + Send),
    ) -> impl Future<Output = Result<FetchStats, TransportError>> + Send;

    /// Closes the connection.
    fn close(self) -> impl Future<Output = ()> + Send;
}
