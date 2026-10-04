//! The owner's side of the in-memory transport: what it serves (every blob
//! ready, unless a test says otherwise) and its suitcases.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ourstick_core::transport::{
    BlobHash, Demand, NetworkReport, Publisher, SuitcaseId, TransportError,
};
use ourstick_core::{AddressHint, NodeId, SignedManifestRecord};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// What the owner serves, shared with the reader's side.
#[derive(Default, Clone)]
pub struct Served {
    pub(crate) blobs: HashMap<BlobHash, Vec<u8>>,
    pub(crate) record: Option<SignedManifestRecord>,
    pub(crate) allowed: HashSet<BlobHash>,
    /// Blobs the owner is still preparing: each ask answers "not ready" and
    /// counts down (ADR 0007).
    pub(crate) not_ready: HashMap<BlobHash, u32>,
    /// The blobs each open suitcase holds ready (they are in `blobs` too).
    pub(crate) suitcases: HashMap<SuitcaseId, HashSet<BlobHash>>,
    pub(crate) next_suitcase: SuitcaseId,
    /// Where what readers ask for goes, once the owner's policy took it.
    pub(crate) demand: Option<UnboundedSender<Demand>>,
    /// When a reader was last told "not ready".
    pub(crate) last_not_ready: Option<std::time::Instant>,
    /// Where readers' sealed reports go, once the owner's side took them
    /// (ADR 0015).
    pub(crate) reports: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    /// The owner cannot be reached for reports (a test of a lost report).
    pub unreachable_for_reports: bool,
    /// Fetches going on now, the most at once, every blob asked for in
    /// order, and how many others were going on when each started.
    pub fetching_now: usize,
    pub most_fetching: usize,
    pub asked: Vec<BlobHash>,
    pub others_at_start: HashMap<BlobHash, usize>,
}

/// Cheap to clone: clones share what is served and the cache folder.
#[derive(Clone)]
pub struct MemoryPublisher {
    pub(crate) served: Arc<Mutex<Served>>,
    pub(crate) staging: Arc<tempfile::TempDir>,
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
    async fn publish(
        &self,
        record: SignedManifestRecord,
        manifest_blob: Vec<u8>,
        allowed: HashSet<BlobHash>,
    ) -> Result<(), TransportError> {
        let manifest = *blake3::hash(&manifest_blob).as_bytes();
        let mut served = self.served.lock().unwrap();
        served.blobs.insert(manifest, manifest_blob);
        served.record = Some(record);
        served.allowed = allowed;
        served.allowed.insert(manifest);
        Ok(())
    }
    fn activity(&self) -> tokio::sync::watch::Receiver<ourstick_core::transport::ReaderActivity> {
        tokio::sync::watch::channel(Default::default()).1
    }
    async fn open_suitcase(&self) -> Result<(SuitcaseId, PathBuf), TransportError> {
        let id = {
            let mut served = self.served.lock().unwrap();
            served.next_suitcase += 1;
            let id = served.next_suitcase;
            served.suitcases.insert(id, HashSet::new());
            id
        };
        let dir = self.staging.path().join("suitcases").join(id.to_string());
        std::fs::create_dir_all(&dir)?;
        Ok((id, dir))
    }
    async fn add_to_suitcase(
        &self,
        suitcase: SuitcaseId,
        blob_file: &Path,
        expected: &BlobHash,
    ) -> Result<bool, TransportError> {
        let bytes = std::fs::read(blob_file)?;
        let hash = *blake3::hash(&bytes).as_bytes();
        let mut served = self.served.lock().unwrap();
        let Some(ready) = served.suitcases.get_mut(&suitcase) else {
            return Err(TransportError::Storage(format!("no suitcase {suitcase}")));
        };
        if hash != *expected {
            return Ok(false);
        }
        ready.insert(hash);
        served.blobs.insert(hash, bytes);
        Ok(true)
    }
    async fn evict(&self, suitcase: SuitcaseId) -> Result<(), TransportError> {
        let mut served = self.served.lock().unwrap();
        let Some(gone) = served.suitcases.remove(&suitcase) else {
            return Ok(());
        };
        for hash in gone {
            if !served.suitcases.values().any(|ready| ready.contains(&hash)) {
                served.blobs.remove(&hash);
            }
        }
        Ok(())
    }
    fn readers_waiting(&self, window: std::time::Duration) -> bool {
        self.served
            .lock()
            .unwrap()
            .last_not_ready
            .is_some_and(|at| at.elapsed() < window)
    }
    fn take_reports(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<u8>>> {
        let mut served = self.served.lock().unwrap();
        if served.reports.is_some() {
            return None;
        }
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        served.reports = Some(sender);
        Some(receiver)
    }
    fn take_demand(&self) -> Option<UnboundedReceiver<Demand>> {
        let mut served = self.served.lock().unwrap();
        if served.demand.is_some() {
            return None;
        }
        let (sender, receiver) = unbounded_channel();
        served.demand = Some(sender);
        Some(receiver)
    }
    async fn shutdown(self) {}
}
