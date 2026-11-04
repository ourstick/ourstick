//! Moving records and blobs between peers.
//!
//! The owner side is a [`Publisher`], the reader side a [`Fetcher`] that opens
//! [`PeerConnection`]s. Nothing outside this module imports iroh or iroh-blobs:
//! the implementation in [`iroh`] can be replaced without touching the rest of
//! the core (ADR 0005).

pub mod diagnosis;
pub mod iroh;
mod published;

use std::future::Future;
use std::ops::ControlFlow;
use std::path::Path;

use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId, Ticket};

pub use diagnosis::{describe_network, relay_reasons, NetworkReport};
pub use published::{hand_over, PublishedVersion};

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
    #[error("the owner sent more data than it said a file has")]
    TooBig,
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
    /// Readers are sent to slowly: OurStick gives way to another program
    /// using a disk their files are read from (ADR 0013; ADR 0017,
    /// amendment 1).
    pub giving_way: bool,
    /// A reader on this computer's network stays on the relay because macOS
    /// declines OurStick the local network here (ADR 0020).
    pub local_network_declined: bool,
}

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
    /// The owner's own store could not serve `hash` (ADR 0017, rule 4): its
    /// file `changed` since the catalogue (a group did not match the tree),
    /// or it could not be read (a disk asleep or gone, a linked folder
    /// removed). It is not ready from then on, until the owner serves it
    /// again.
    NotServed { hash: BlobHash, changed: bool },
}

/// One file the owner's store serves (ADR 0017), as the catalogue recorded
/// it: the index of its entry in the version's manifest (its path and size
/// are the entry's, ADR 0019; where it is on disk comes from the stick's
/// layout when it is asked for) and its plaintext hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedFile {
    pub index: u32,
    pub plaintext_hash: [u8; 32],
}

/// What the owner's store serves: the current version's manifest, its
/// files by blob hash, and where the stick's roots are.
#[derive(Debug, Clone)]
pub struct ServedFiles {
    pub layout: crate::layout::StickLayout,
    pub manifest: std::sync::Arc<crate::manifest::Manifest>,
    pub files: std::collections::HashMap<BlobHash, ServedFile>,
}

impl ServedFiles {
    /// The entry of a served file.
    pub fn entry(&self, file: &ServedFile) -> &crate::manifest::Entry {
        &self.manifest.entries[file.index as usize]
    }
}

/// The owner's side: serves the current record, its manifest, and its
/// files' blobs, made from the files when asked (ADR 0017).
pub trait Publisher: Send + Sync {
    /// This device's node id, for tickets.
    fn node_id(&self) -> NodeId;

    /// Where this node can be reached now, for tickets.
    fn address_hints(&self) -> Vec<AddressHint>;

    /// What this node found out about its network, once measured.
    fn network_report(&self) -> Option<NetworkReport>;

    /// The stick's cache folder: the latest manifest and the outboards.
    fn cache_dir(&self) -> &Path;

    /// Starts serving `record`, with `manifest_blob` (the blob it names,
    /// served from memory) and the blobs of `manifest`, its decoded content;
    /// others stop being served at once. A blob of it not served yet (its
    /// file not made ready) is answered "not ready". The owner's files
    /// keeper is handed `manifest` (ADR 0019); the manifest kept is
    /// returned: the one kept before if it is the same version, so there
    /// is one copy of it, not two.
    fn publish(
        &self,
        record: SignedManifestRecord,
        manifest_blob: Vec<u8>,
        manifest: std::sync::Arc<crate::manifest::Manifest>,
    ) -> impl Future<Output = Result<std::sync::Arc<crate::manifest::Manifest>, TransportError>> + Send;

    /// The version published last, from now on: what the owner's files
    /// keeper serves (ADR 0019).
    fn published(&self) -> tokio::sync::watch::Receiver<Option<PublishedVersion>>;

    /// Serves the blobs of `files` from the owner's own store from now on,
    /// made from the files when they are asked for, in place of what it
    /// served before (ADR 0017). Their outboards are in the stick's cache.
    /// Nothing by default.
    fn serve_files(&self, _cipher: std::sync::Arc<dyn crate::cipher::Cipher>, _files: ServedFiles) {
    }

    /// Serves these files too, in the version given last to
    /// [`Publisher::serve_files`] (files made ready since), if `manifest`
    /// is still that version's: indexes of another version name other
    /// files. Nothing by default.
    fn serve_more(
        &self,
        _manifest: &std::sync::Arc<crate::manifest::Manifest>,
        _files: Vec<(BlobHash, ServedFile)>,
    ) {
    }

    /// Whether a reader was told "not ready" within the last `window`: a
    /// reader waits on the owner, whose reading for it then goes first on
    /// the disk (ADR 0013).
    fn readers_waiting(&self, window: std::time::Duration) -> bool;

    /// What readers ask for, from now on. Only the first call gets it.
    fn take_demand(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<Demand>>;

    /// Readers' reports on this stick, sealed, from now on (ADR 0015): the
    /// owner's side opens them with the stick's key. Not demand, not
    /// activity: a channel of its own. Only the first call gets it.
    fn take_reports(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<u8>>>;

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

    /// The network changed (a phone hears it from the system, ADR 0027):
    /// paths to peers are looked for again at once. Harmless otherwise.
    fn network_change(&self) -> impl Future<Output = ()> + Send;

    /// Connects to the owner named in `ticket`.
    fn connect(
        &self,
        ticket: &Ticket,
    ) -> impl Future<Output = Result<Self::Connection, TransportError>> + Send;

    /// Sends a sealed reader report to the owner named in `ticket` (ADR
    /// 0015) and waits until the owner has taken it; an error if it was not.
    /// Best effort: the caller bounds it.
    fn report(
        &self,
        ticket: &Ticket,
        sealed: &[u8],
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

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
    /// in use, [`MANIFEST_BATCH`] is used from then on. [`MANIFEST_BATCH`]
    /// itself is emptied: every manifest kept goes.
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

    /// Whether the owner, on this computer's network, stays on the relay
    /// because macOS declines OurStick the local network here (ADR 0020).
    fn local_network_declined(&self) -> bool {
        false
    }

    /// Asks the owner for the latest signed record of `stick_id`. The record is
    /// not verified here.
    fn fetch_record(
        &self,
        stick_id: &StickId,
    ) -> impl Future<Output = Result<SignedManifestRecord, TransportError>> + Send;

    /// Downloads the missing parts of a blob into the local store, verifying
    /// every chunk against `hash`. `on_progress` gets the bytes received so far
    /// in this fetch, and says whether to go on (after waiting a while, which
    /// slows the sender through flow control) or to cancel. A blob that is,
    /// or becomes, longer than `largest` bytes fails with
    /// [`TransportError::TooBig`] as soon as that shows: its hash covers it,
    /// but the owner said it was smaller.
    fn fetch_blob(
        &self,
        hash: &BlobHash,
        largest: u64,
        on_progress: &mut (dyn FnMut(u64) -> Pace + Send),
    ) -> impl Future<Output = Result<FetchStats, TransportError>> + Send;

    /// Closes the connection.
    fn close(self) -> impl Future<Output = ()> + Send;
}
