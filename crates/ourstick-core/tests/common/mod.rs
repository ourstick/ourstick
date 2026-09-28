//! Two peers on one machine: an owner that shares a folder and a reader that
//! downloads it, over real iroh endpoints on loopback (no relays, no internet).

#![allow(dead_code)] // each test binary uses a different subset

pub mod memory;

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ourstick_core::clock::FixedClock;
use ourstick_core::download::{
    download, DownloadError, DownloadEvent, DownloadReport, RelayConsent, RelayPolicy,
    RelayQuestion,
};
use ourstick_core::share::{publish_folder, Publication};
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, IrohPublisher, Network};
use ourstick_core::transport::Publisher;
use ourstick_core::{
    CipherId, ManifestHint, MemoryStore, OwnedStick, StickKey, StickSigningKey, Store, Ticket,
};
use rand::RngCore;
use tempfile::TempDir;

/// For transfers that must never reach the relay question (loopback is
/// always direct).
pub struct NeverAsked;

impl RelayConsent for NeverAsked {
    async fn allow_relay(&self, question: RelayQuestion) -> bool {
        panic!("unexpected relay question: {question:?}")
    }
}

pub fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Bytes that do not compress and differ per `seed`.
pub fn noise(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    let mut state = u64::from(seed) + 1;
    for byte in &mut bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
    bytes
}

pub struct Owner {
    pub publisher: IrohPublisher,
    pub store: Arc<MemoryStore>,
    pub clock: FixedClock,
    pub signing_key: StickSigningKey,
    pub stick_key: StickKey,
    pub folder: TempDir,
    pub cache: TempDir,
}

impl Owner {
    /// An owner sharing a folder with `files`, not yet published, on loopback.
    pub async fn new(files: &[(&str, &[u8])]) -> Self {
        Self::on(Network::LocalOnly, files).await
    }

    /// Like [`Owner::new`], on the given network.
    pub async fn on(network: Network, files: &[(&str, &[u8])]) -> Self {
        let folder = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            write_file(folder.path(), path, contents);
        }
        let signing_key = StickSigningKey::from_bytes(&random_key());
        let store = Arc::new(MemoryStore::new());
        store
            .add_owned_stick(&OwnedStick::new(
                signing_key.stick_id(),
                "test stick",
                folder.path(),
                CipherId::XChaCha20Poly1305,
                1,
            ))
            .unwrap();
        let cache = tempfile::tempdir().unwrap();
        let config =
            IrohConfig::new(random_key(), network).with_gc_interval(Duration::from_millis(100));
        let publisher = IrohPublisher::start(&config, signing_key.stick_id(), cache.path())
            .await
            .unwrap();
        Self {
            publisher,
            store,
            clock: FixedClock::new(1_000),
            signing_key,
            stick_key: StickKey::from_bytes(random_key()),
            folder,
            cache,
        }
    }

    pub async fn publish(&self) -> Publication {
        publish_folder(
            &self.publisher,
            self.store.clone(),
            &self.clock,
            &self.signing_key,
            &self.stick_key,
        )
        .await
        .unwrap()
    }

    pub fn ticket(&self, publication: &Publication) -> Ticket {
        Ticket {
            stick_id: self.signing_key.stick_id(),
            node_id: self.publisher.node_id(),
            manifest_hint: ManifestHint {
                seq: publication.seq,
                hash: publication.manifest_hash,
            },
            address_hints: self.publisher.address_hints(),
            stick_key: self.stick_key.clone(),
        }
    }

    /// Path of a blob's data file in the owner's cache (blobs over 16 KiB).
    pub fn cached_data_file(&self, hash: &[u8; 32]) -> PathBuf {
        self.cache
            .path()
            .join("blobs/data")
            .join(format!("{}.data", data_encoding::HEXLOWER.encode(hash)))
    }
}

pub fn write_file(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

pub struct Reader {
    pub network: Network,
    pub store: Arc<MemoryStore>,
    pub clock: FixedClock,
    pub blobs: TempDir,
    pub output: TempDir,
}

impl Reader {
    pub fn new() -> Self {
        Self::on(Network::LocalOnly)
    }

    pub fn on(network: Network) -> Self {
        Self {
            network,
            store: Arc::new(MemoryStore::new()),
            clock: FixedClock::new(2_000),
            blobs: tempfile::tempdir().unwrap(),
            output: tempfile::tempdir().unwrap(),
        }
    }

    pub async fn fetcher(&self) -> IrohFetcher {
        let config = IrohConfig::new(random_key(), self.network);
        IrohFetcher::start(&config, self.blobs.path())
            .await
            .unwrap()
    }

    /// One download with a fresh fetcher, shut down afterwards (store kept).
    pub async fn download(
        &self,
        ticket: &Ticket,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<DownloadReport, DownloadError> {
        use ourstick_core::transport::Fetcher;
        let fetcher = self.fetcher().await;
        let result = download(
            &fetcher,
            self.store.clone(),
            &self.clock,
            ticket,
            self.output.path(),
            &RelayPolicy::default(),
            &NeverAsked,
            on_event,
        )
        .await;
        fetcher.shutdown(true).await.unwrap();
        result
    }

    pub fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.output.path().join(relative)).unwrap()
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.output.path().join(relative).exists()
    }
}

pub fn keep_going(_: DownloadEvent<'_>) -> ControlFlow<()> {
    ControlFlow::Continue(())
}
