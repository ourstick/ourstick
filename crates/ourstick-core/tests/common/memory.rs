//! An in-memory transport (CLAUDE.md rule 2) for what real iroh on loopback
//! cannot produce: a reader that sends over the relay while data arrives
//! directly, a network with no direct path, a direct path lost mid-transfer.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ourstick_core::transport::{
    is_batch_name, BlobHash, Demand, FetchStats, Fetcher, NetworkReport, Pace, PathKind, PathUsage,
    PeerConnection, TransportError, MANIFEST_BATCH,
};
use ourstick_core::{SignedManifestRecord, StickId, Ticket};

pub use super::owner_in_memory::{MemoryPublisher, Served};

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
    /// Once this many bytes have arrived, the connection is lost, once.
    pub lose_connection_after: Option<u64>,
}

/// The reader's blobs (complete or partial), one map per batch as with iroh,
/// and what arrived over which path, over all connections.
pub struct Local {
    batches: HashMap<String, HashMap<BlobHash, Vec<u8>>>,
    current: String,
    pub usage: PathUsage,
    /// Every batch dropped, in order.
    pub dropped: Vec<String>,
}

impl Default for Local {
    fn default() -> Self {
        Self {
            batches: HashMap::new(),
            current: MANIFEST_BATCH.to_owned(),
            usage: PathUsage::default(),
            dropped: Vec::new(),
        }
    }
}

impl Local {
    /// The blobs of the batch in use.
    fn blobs(&mut self) -> &mut HashMap<BlobHash, Vec<u8>> {
        self.batches.entry(self.current.clone()).or_default()
    }
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
        local
            .batches
            .values()
            .flat_map(|blobs| blobs.values())
            .map(|bytes| bytes.len() as u64)
            .sum()
    }

    /// The batches that hold blobs now.
    pub fn batch_names(&self) -> Vec<String> {
        let local = self.local.lock().unwrap();
        let mut names: Vec<_> = local
            .batches
            .iter()
            .filter(|(_, blobs)| !blobs.is_empty())
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
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
    /// Its id, as the owner sees it.
    id: u64,
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
        let id = {
            let mut connects = self.connects.lock().unwrap();
            *connects += 1;
            u64::from(*connects)
        };
        // The reader starts out sending over the relay, as with iroh.
        Ok(MemoryConnection {
            id,
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
            .blobs()
            .get(hash)
            .cloned()
            .ok_or_else(|| TransportError::Storage("not local".into()))
    }
    async fn export_blob(&self, hash: &BlobHash, target: &Path) -> Result<(), TransportError> {
        std::fs::write(target, self.read_blob(hash).await?)?;
        Ok(())
    }
    async fn local_bytes(&self, hash: &BlobHash) -> Result<u64, TransportError> {
        let mut local = self.local.lock().unwrap();
        Ok(local
            .blobs()
            .get(hash)
            .map_or(0, |bytes| bytes.len() as u64))
    }
    async fn use_batch(&self, name: &str) -> Result<(), TransportError> {
        assert!(is_batch_name(name), "{name:?}");
        self.local.lock().unwrap().current = name.to_owned();
        Ok(())
    }
    async fn drop_batch(&self, name: &str) -> Result<(), TransportError> {
        let mut local = self.local.lock().unwrap();
        local.batches.remove(name);
        local.dropped.push(name.to_owned());
        if local.current == name {
            local.current = MANIFEST_BATCH.to_owned();
        }
        Ok(())
    }
    async fn keep_only_batches(&self, keep: &[String]) -> Result<(), TransportError> {
        let mut local = self.local.lock().unwrap();
        let current = local.current.clone();
        local
            .batches
            .retain(|name, _| name == MANIFEST_BATCH || *name == current || keep.contains(name));
        Ok(())
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
            let mut served = self.served.lock().unwrap();
            if !served.allowed.contains(hash) {
                return Err(TransportError::Refused);
            }
            let preparing = match served.not_ready.get_mut(hash) {
                Some(left @ 1..) => {
                    *left -= 1;
                    true
                }
                _ => false,
            };
            // Allowed but not here yet: the owner is preparing it.
            let blob = served.blobs.get(hash).cloned().filter(|_| !preparing);
            if let Some(demand) = &served.demand {
                demand
                    .send(Demand::Asked {
                        connection: self.id,
                        hash: *hash,
                        served: blob.is_some(),
                    })
                    .ok();
            }
            if blob.is_none() {
                served.last_not_ready = Some(std::time::Instant::now());
            }
            blob.ok_or(TransportError::NotReady)?
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
            .blobs()
            .get(hash)
            .map_or(0, Vec::len);
        let mut received = 0u64;
        for piece in blob[already_local..].chunks(1024 * 1024) {
            {
                let mut link = self.link.lock().unwrap();
                if link
                    .lose_connection_after
                    .is_some_and(|limit| link.delivered >= limit)
                {
                    link.lose_connection_after = None;
                    return Err(TransportError::Interrupted(
                        "read: connection lost".to_owned(),
                    ));
                }
            }
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
                    .blobs()
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
    async fn close(self) {
        if let Some(demand) = &self.served.lock().unwrap().demand {
            demand
                .send(Demand::Closed {
                    connection: self.id,
                })
                .ok();
        }
    }
}

// Each test binary uses a different subset.
#[allow(unused_imports)]
pub use super::published::{
    publish_files, publish_one_file, publish_versions, Published, Versions,
};
