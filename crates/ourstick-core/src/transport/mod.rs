//! Moving records and blobs between peers.
//!
//! The owner side is a [`Publisher`], the reader side a [`Fetcher`] that opens
//! [`PeerConnection`]s. Nothing outside this module imports iroh or iroh-blobs:
//! the implementation in [`iroh`] can be replaced without touching the rest of
//! the core (ADR 0005).

pub mod diagnosis;
pub mod iroh;

use std::collections::HashSet;
use std::future::Future;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId, Ticket};

pub use diagnosis::{describe_network, relay_reasons, NetworkReport};

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
    /// The part of `direct` that came over IPv6.
    pub direct_ipv6: u64,
}

/// What a fetch does after reporting progress: `Continue(wait)` goes on after
/// `wait` (zero: at once), `Break` cancels.
pub type Pace = ControlFlow<(), std::time::Duration>;

/// Go on at once.
pub const GO: Pace = ControlFlow::Continue(std::time::Duration::ZERO);

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
    #[error("the owner is still preparing this data; asking again shortly")]
    NotReady,
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

/// What readers are doing, as the owner sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReaderActivity {
    /// Readers with a transfer in progress (connections, strictly).
    pub readers: usize,
    /// How fast data leaves, over the last ten seconds.
    pub bytes_per_second: u64,
    /// Readers wait while the owner gets their files ready (they were told
    /// "not ready", and nothing was sent since).
    pub preparing: bool,
    /// Readers reach this owner only through the relay: one stayed on it
    /// for a while, and none has ever come directly since sharing started.
    /// Most likely a firewall drops what others send (see
    /// `docs/protocol.md`, "Reader paths").
    pub only_relayed: bool,
}

/// A suitcase (ADR 0007): a store of blobs that come and go together. Ids
/// are never reused while the publisher runs.
pub type SuitcaseId = u64;

/// What readers ask for, as the owner's policy needs it (ADR 0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demand {
    /// `connection` asked for `hash`, an allowed blob; `served` says whether
    /// a store held it ready.
    Asked {
        connection: u64,
        hash: BlobHash,
        served: bool,
    },
    /// `connection` closed.
    Closed { connection: u64 },
}

/// The owner's side: serves the current record, its manifest, and the
/// blobs its suitcases hold (ADR 0007).
pub trait Publisher: Send + Sync {
    /// This device's node id, for tickets.
    fn node_id(&self) -> NodeId;

    /// Where this node can be reached now, for tickets.
    fn address_hints(&self) -> Vec<AddressHint>;

    /// What this node found out about its network, once measured.
    fn network_report(&self) -> Option<NetworkReport>;

    /// The stick's cache folder: its suitcases, and the latest manifest.
    fn cache_dir(&self) -> &Path;

    /// Starts serving `record`, with `manifest_blob` (the blob it names,
    /// served from memory) and the blobs in `allowed`; others stop being
    /// served at once. An allowed blob that no suitcase holds ready is
    /// answered "not ready" (ADR 0007).
    fn publish(
        &self,
        record: SignedManifestRecord,
        manifest_blob: Vec<u8>,
        allowed: HashSet<BlobHash>,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Opens a new, empty suitcase, and the folder its blob files go in.
    fn open_suitcase(
        &self,
    ) -> impl Future<Output = Result<(SuitcaseId, PathBuf), TransportError>> + Send;

    /// Adds the blob in `blob_file`, a file in the suitcase's folder that
    /// stays there until the suitcase goes, and makes it ready to serve if
    /// and only if its hash is `expected` (ADR 0007: a blob is served only
    /// if its hash is the manifest's). Returns whether it is.
    fn add_to_suitcase(
        &self,
        suitcase: SuitcaseId,
        blob_file: &Path,
        expected: &BlobHash,
    ) -> impl Future<Output = Result<bool, TransportError>> + Send;

    /// Stops serving from `suitcase`, waits until no transfer from it is in
    /// progress, then closes it and removes its folder.
    fn evict(
        &self,
        suitcase: SuitcaseId,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Whether a reader was told "not ready" within the last `window`: a
    /// reader waits on the owner (ADR 0007), which then puts preparing
    /// suitcases before reading the folder.
    fn readers_waiting(&self, window: std::time::Duration) -> bool;

    /// What readers ask for, from now on. Only the first call gets it.
    fn take_demand(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<Demand>>;

    /// What readers are doing, updated about twice a second while it
    /// changes.
    fn activity(&self) -> tokio::sync::watch::Receiver<ReaderActivity>;

    /// Stops serving and closes the cache cleanly.
    fn shutdown(self) -> impl Future<Output = ()> + Send;
}

/// The batch a fetcher starts with, and the one the manifest is fetched into.
pub const MANIFEST_BATCH: &str = "manifest";

/// Whether `name` may name a batch: ASCII letters, digits and `-`, never empty.
pub fn is_batch_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The reader's side: a local blob store and connections to owners.
pub trait Fetcher: Send + Sync {
    type Connection: PeerConnection;

    /// What this node found out about its network, once measured.
    fn network_report(&self) -> Option<NetworkReport>;

    /// Which relay this node uses, for the user (e.g. when asking whether a
    /// transfer may go through it).
    fn relay_description(&self) -> String;

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

    /// Blobs are kept in batches, each its own store (ADR 0007): from now on
    /// blobs are fetched into, and read from, the batch `name` (made if new;
    /// an earlier run's partial blobs in it are resumed). `name` is made of
    /// ASCII letters, digits and `-`.
    fn use_batch(&self, name: &str) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Deletes the batch `name` and its blobs, freeing their space. If it was
    /// in use, [`MANIFEST_BATCH`] is used from then on.
    fn drop_batch(&self, name: &str) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Deletes every batch not named in `keep` (and not [`MANIFEST_BATCH`]),
    /// e.g. those of an older version of the stick.
    fn keep_only_batches(
        &self,
        keep: &[String],
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Closes the local store cleanly, keeping partial blobs for a later resume
    /// (`keep = true`) or deleting every batch (`keep = false`).
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

    /// Whether a direct (non-relay) path to the peer is open right now.
    fn has_direct_path(&self) -> bool;

    /// Asks the owner for the latest signed record of `stick_id`. The record is
    /// not verified here.
    fn fetch_record(
        &self,
        stick_id: &StickId,
    ) -> impl Future<Output = Result<SignedManifestRecord, TransportError>> + Send;

    /// Downloads the missing parts of a blob into the local store, verifying
    /// every chunk against `hash`. `on_progress` gets the bytes received so far
    /// in this fetch, and says whether to go on (after waiting a while, which
    /// slows the sender through flow control) or to cancel.
    fn fetch_blob(
        &self,
        hash: &BlobHash,
        on_progress: &mut (dyn FnMut(u64) -> Pace + Send),
    ) -> impl Future<Output = Result<FetchStats, TransportError>> + Send;

    /// Closes the connection.
    fn close(self) -> impl Future<Output = ()> + Send;
}
