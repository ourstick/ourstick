//! Regression test for the first real transfer (PC to a MacBook on a phone
//! hotspot): every progress line said "relay", yet 4.6 GB of 4.6 GB arrived
//! over a direct path. With QUIC multipath each side picks the path it sends
//! on; the reader sends almost nothing and stayed on the relay while the owner
//! sent everything directly.
//!
//! That asymmetry cannot be produced with real iroh on loopback, so this test
//! uses an in-memory transport whose connection *sends* over the relay
//! (`path()` is `Relay`) while every received byte arrives *directly*
//! (`usage().direct` grows). The reader must end up reporting "direct".

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadEvent};
use ourstick_core::share::publish_folder;
use ourstick_core::transport::{
    BlobHash, FetchStats, Fetcher, PathKind, PathUsage, PeerConnection, Publisher, TransportError,
};
use ourstick_core::{
    AddressHint, CipherId, ManifestHint, MemoryStore, NodeId, OwnedStick, SignedManifestRecord,
    StickId, StickKey, StickSigningKey, Store, Ticket,
};

/// What the owner serves, shared with the reader's side.
#[derive(Default)]
struct Served {
    blobs: HashMap<BlobHash, Vec<u8>>,
    record: Option<SignedManifestRecord>,
    allowed: HashSet<BlobHash>,
}

struct MemoryPublisher {
    served: Arc<Mutex<Served>>,
    staging: tempfile::TempDir,
}

impl Publisher for MemoryPublisher {
    fn node_id(&self) -> NodeId {
        NodeId([7; 32])
    }
    fn address_hints(&self) -> Vec<AddressHint> {
        Vec::new()
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
    async fn has_blob(&self, hash: &BlobHash) -> Result<bool, TransportError> {
        Ok(self.served.lock().unwrap().blobs.contains_key(hash))
    }
    async fn shutdown(self) {}
}

/// The reader's local blobs, and what arrived over which kind of path.
#[derive(Default)]
struct Local {
    blobs: HashMap<BlobHash, Vec<u8>>,
    usage: PathUsage,
}

struct MemoryFetcher {
    served: Arc<Mutex<Served>>,
    local: Arc<Mutex<Local>>,
}

struct MemoryConnection {
    served: Arc<Mutex<Served>>,
    local: Arc<Mutex<Local>>,
    path: tokio::sync::watch::Receiver<PathKind>,
}

impl Fetcher for MemoryFetcher {
    type Connection = MemoryConnection;

    async fn connect(&self, _ticket: &Ticket) -> Result<MemoryConnection, TransportError> {
        // The reader's own (sending) path is the relay, and stays there.
        let (_sender, path) = tokio::sync::watch::channel(PathKind::Relay);
        Ok(MemoryConnection {
            served: self.served.clone(),
            local: self.local.clone(),
            path,
        })
    }
    async fn read_blob(&self, hash: &BlobHash) -> Result<Vec<u8>, TransportError> {
        let local = self.local.lock().unwrap();
        local
            .blobs
            .get(hash)
            .cloned()
            .ok_or_else(|| TransportError::Storage("not local".into()))
    }
    async fn export_blob(&self, hash: &BlobHash, target: &Path) -> Result<(), TransportError> {
        let bytes = self.read_blob(hash).await?;
        std::fs::write(target, bytes)?;
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
        *self.path.borrow()
    }
    fn path_changes(&self) -> tokio::sync::watch::Receiver<PathKind> {
        self.path.clone()
    }
    fn usage(&self) -> PathUsage {
        self.local.lock().unwrap().usage
    }
    async fn fetch_record(
        &self,
        _stick_id: &StickId,
    ) -> Result<SignedManifestRecord, TransportError> {
        self.served
            .lock()
            .unwrap()
            .record
            .clone()
            .ok_or(TransportError::NoRecord)
    }
    /// Delivers the blob in 1 MiB pieces, every one of them over a direct path.
    async fn fetch_blob(
        &self,
        hash: &BlobHash,
        on_progress: &mut (dyn FnMut(u64) -> ControlFlow<()> + Send),
    ) -> Result<FetchStats, TransportError> {
        let bytes = {
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
        let mut received = 0u64;
        for piece in bytes.chunks(1024 * 1024) {
            received += piece.len() as u64;
            self.local.lock().unwrap().usage.direct += piece.len() as u64;
            if on_progress(received).is_break() {
                return Err(TransportError::Cancelled);
            }
        }
        self.local.lock().unwrap().blobs.insert(*hash, bytes);
        Ok(FetchStats {
            already_local: 0,
            received,
        })
    }
    async fn close(self) {}
}

fn random_key() -> [u8; 32] {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_reports_direct_when_data_arrives_directly_while_it_sends_via_relay() {
    // Owner: a folder with one 20 MiB file, published into memory.
    let folder = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(folder.path().join("movie.bin"), &data).unwrap();
    let signing_key = StickSigningKey::from_bytes(&random_key());
    let stick_key = StickKey::from_bytes(random_key());
    let owner_store = Arc::new(MemoryStore::new());
    owner_store
        .add_owned_stick(&OwnedStick::new(
            signing_key.stick_id(),
            "test",
            folder.path(),
            CipherId::XChaCha20Poly1305,
            1,
        ))
        .unwrap();
    let served = Arc::new(Mutex::new(Served::default()));
    let publisher = MemoryPublisher {
        served: served.clone(),
        staging: tempfile::tempdir().unwrap(),
    };
    let publication = publish_folder(
        &publisher,
        owner_store,
        &FixedClock::new(1_000),
        &signing_key,
        &stick_key,
    )
    .await
    .unwrap();
    let ticket = Ticket {
        stick_id: signing_key.stick_id(),
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: publication.seq,
            hash: publication.manifest_hash,
        },
        address_hints: Vec::new(),
        stick_key,
    };

    // Reader: remember the last path the download reported.
    let fetcher = MemoryFetcher {
        served,
        local: Arc::new(Mutex::new(Local::default())),
    };
    let output = tempfile::tempdir().unwrap();
    let mut reported: Vec<PathKind> = Vec::new();
    let report = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &ticket,
        output.path(),
        &mut |event| {
            match event {
                DownloadEvent::Connected { path } | DownloadEvent::PathChanged { path } => {
                    reported.push(path);
                }
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        data
    );
    assert_eq!(report.usage.relay, 0);
    assert!(report.usage.direct >= data.len() as u64);
    // It starts on the relay (true for the reader's own sending path) ...
    assert_eq!(reported.first(), Some(&PathKind::Relay));
    // ... but all data came directly, so that is what it must end up saying.
    assert_eq!(
        reported.last(),
        Some(&PathKind::Direct),
        "reported paths: {reported:?}, usage: {:?}",
        report.usage
    );
}
