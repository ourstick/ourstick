//! An in-memory transport (CLAUDE.md rule 2) for what real iroh on loopback
//! cannot produce: a reader that sends over the relay while data arrives
//! directly, a network with no direct path, a direct path lost mid-transfer.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ourstick_core::transport::{
    BlobHash, FetchStats, Fetcher, NetworkReport, Pace, PathKind, PathUsage, PeerConnection,
    Publisher, TransportError,
};
use ourstick_core::{AddressHint, NodeId, SignedManifestRecord, StickId, Ticket};

/// What the owner serves, shared with the reader's side.
#[derive(Default, Clone)]
pub struct Served {
    pub(crate) blobs: HashMap<BlobHash, Vec<u8>>,
    pub(crate) record: Option<SignedManifestRecord>,
    pub(crate) allowed: HashSet<BlobHash>,
}

/// The network between reader and owner.
#[derive(Debug, Default)]
pub struct Link {
    /// A direct path is open; otherwise data goes through the relay.
    pub direct: bool,
    /// Once this many bytes have arrived, the direct path is lost for good.
    pub lose_direct_after: Option<u64>,
    /// Once this many bytes have arrived, a direct path opens (hole punching
    /// succeeded while data went through the relay).
    pub direct_after: Option<u64>,
    /// Hole punching succeeds during the first fetch (the manifest).
    pub punch_during_first_fetch: bool,
    /// Bytes delivered so far, over any path.
    pub delivered: u64,
}

pub struct MemoryPublisher {
    pub(crate) served: Arc<Mutex<Served>>,
    pub(crate) staging: tempfile::TempDir,
}

impl Publisher for MemoryPublisher {
    fn node_id(&self) -> NodeId {
        NodeId([7; 32])
    }
    fn address_hints(&self) -> Vec<AddressHint> {
        Vec::new()
    }
    fn network_report(&self) -> Option<NetworkReport> {
        None
    }
    fn cache_dir(&self) -> &Path {
        self.staging.path()
    }
    fn staging_dir(&self) -> &Path {
        self.staging.path()
    }
    async fn import_blob(&self, blob_file: &Path) -> Result<BlobHash, TransportError> {
        let bytes = std::fs::read(blob_file)?;
        let hash = *blake3::hash(&bytes).as_bytes();
        self.served.lock().unwrap().blobs.insert(hash, bytes);
        Ok(hash)
    }
    async fn publish(
        &self,
        record: SignedManifestRecord,
        allowed: HashSet<BlobHash>,
    ) -> Result<(), TransportError> {
        let mut served = self.served.lock().unwrap();
        served.record = Some(record);
        served.allowed = allowed;
        Ok(())
    }
    async fn keep_blob(&self, hash: &BlobHash) -> Result<bool, TransportError> {
        self.has_blob(hash).await
    }
    async fn has_blob(&self, hash: &BlobHash) -> Result<bool, TransportError> {
        Ok(self.served.lock().unwrap().blobs.contains_key(hash))
    }
    async fn shutdown(self) {}
}

/// The reader's blobs (complete or partial) and what arrived over which path,
/// over all connections.
#[derive(Default)]
pub struct Local {
    blobs: HashMap<BlobHash, Vec<u8>>,
    pub usage: PathUsage,
}

pub struct MemoryFetcher {
    pub served: Arc<Mutex<Served>>,
    pub local: Arc<Mutex<Local>>,
    pub link: Arc<Mutex<Link>>,
    pub connects: Arc<Mutex<u32>>,
}

impl MemoryFetcher {
    /// Blob bytes stored locally, complete or partial.
    pub fn local_bytes_total(&self) -> u64 {
        let local = self.local.lock().unwrap();
        local.blobs.values().map(|bytes| bytes.len() as u64).sum()
    }

    pub fn new(served: Arc<Mutex<Served>>, link: Link) -> Self {
        Self {
            served,
            local: Arc::default(),
            link: Arc::new(Mutex::new(link)),
            connects: Arc::default(),
        }
    }
}

pub struct MemoryConnection {
    served: Arc<Mutex<Served>>,
    local: Arc<Mutex<Local>>,
    link: Arc<Mutex<Link>>,
    /// The path this reader sends on.
    path: Arc<Mutex<PathKind>>,
    /// What arrived over which path on this connection, as with iroh.
    usage: Mutex<PathUsage>,
}

impl Fetcher for MemoryFetcher {
    type Connection = MemoryConnection;

    fn network_report(&self) -> Option<NetworkReport> {
        None
    }
    fn relay_description(&self) -> String {
        "the test relay".to_owned()
    }
    async fn connect(&self, _ticket: &Ticket) -> Result<MemoryConnection, TransportError> {
        *self.connects.lock().unwrap() += 1;
        // The reader starts out sending over the relay, as with iroh.
        Ok(MemoryConnection {
            served: self.served.clone(),
            local: self.local.clone(),
            link: self.link.clone(),
            path: Arc::new(Mutex::new(PathKind::Relay)),
            usage: Mutex::default(),
        })
    }
    async fn read_blob(&self, hash: &BlobHash) -> Result<Vec<u8>, TransportError> {
        self.local
            .lock()
            .unwrap()
            .blobs
            .get(hash)
            .cloned()
            .ok_or_else(|| TransportError::Storage("not local".into()))
    }
    async fn export_blob(&self, hash: &BlobHash, target: &Path) -> Result<(), TransportError> {
        std::fs::write(target, self.read_blob(hash).await?)?;
        Ok(())
    }
    async fn local_bytes(&self, hash: &BlobHash) -> Result<u64, TransportError> {
        let local = self.local.lock().unwrap();
        Ok(local.blobs.get(hash).map_or(0, |bytes| bytes.len() as u64))
    }
    async fn shutdown(self, _keep: bool) -> Result<(), TransportError> {
        Ok(())
    }
}

impl PeerConnection for MemoryConnection {
    fn path(&self) -> PathKind {
        *self.path.lock().unwrap()
    }
    fn path_changes(&self) -> tokio::sync::watch::Receiver<PathKind> {
        tokio::sync::watch::channel(self.path()).1
    }
    fn usage(&self) -> PathUsage {
        *self.usage.lock().unwrap()
    }
    fn has_direct_path(&self) -> bool {
        self.link.lock().unwrap().direct
    }
    async fn fetch_record(
        &self,
        _stick_id: &StickId,
    ) -> Result<SignedManifestRecord, TransportError> {
        let served = self.served.lock().unwrap();
        served.record.clone().ok_or(TransportError::NoRecord)
    }
    /// Delivers the missing part of the blob in 1 MiB pieces, each over the
    /// direct path if one is open, else through the relay.
    async fn fetch_blob(
        &self,
        hash: &BlobHash,
        on_progress: &mut (dyn FnMut(u64) -> Pace + Send),
    ) -> Result<FetchStats, TransportError> {
        let blob = {
            let served = self.served.lock().unwrap();
            if !served.allowed.contains(hash) {
                return Err(TransportError::Refused);
            }
            served
                .blobs
                .get(hash)
                .cloned()
                .ok_or(TransportError::Refused)?
        };
        {
            let mut link = self.link.lock().unwrap();
            if link.punch_during_first_fetch {
                link.punch_during_first_fetch = false;
                link.direct = true;
                *self.path.lock().unwrap() = PathKind::Direct;
            }
        }
        let already_local = self
            .local
            .lock()
            .unwrap()
            .blobs
            .get(hash)
            .map_or(0, Vec::len);
        let mut received = 0u64;
        for piece in blob[already_local..].chunks(1024 * 1024) {
            let direct = {
                let mut link = self.link.lock().unwrap();
                link.delivered += piece.len() as u64;
                let direct = link.direct;
                if link
                    .lose_direct_after
                    .is_some_and(|limit| link.delivered >= limit)
                {
                    link.direct = false;
                } else if link
                    .direct_after
                    .is_some_and(|limit| link.delivered >= limit)
                {
                    link.direct_after = None;
                    link.direct = true;
                }
                direct
            };
            {
                let mut local = self.local.lock().unwrap();
                let mut usage = self.usage.lock().unwrap();
                for usage in [&mut local.usage, &mut *usage] {
                    if direct {
                        usage.direct += piece.len() as u64;
                    } else {
                        usage.relay += piece.len() as u64;
                    }
                }
                local
                    .blobs
                    .entry(*hash)
                    .or_default()
                    .extend_from_slice(piece);
            }
            received += piece.len() as u64;
            match on_progress(received) {
                ControlFlow::Break(()) => return Err(TransportError::Cancelled),
                ControlFlow::Continue(wait) if !wait.is_zero() => tokio::time::sleep(wait).await,
                ControlFlow::Continue(_) => {}
            }
        }
        Ok(FetchStats {
            already_local: already_local as u64,
            received,
        })
    }
    async fn close(self) {}
}

// Each test binary uses a different subset.
#[allow(unused_imports)]
pub use super::published::{publish_one_file, publish_versions, Published, Versions};
