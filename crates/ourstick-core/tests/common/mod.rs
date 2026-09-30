//! Two peers on one machine: an owner that shares a folder and a reader that
//! downloads it, over real iroh endpoints on loopback (no relays, no internet).

#![allow(dead_code)] // each test binary uses a different subset

pub mod growing;
pub mod memory;
pub mod published;

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ourstick_core::clock::FixedClock;
use ourstick_core::download::{
    download, DownloadError, DownloadEvent, DownloadReport, RelayPolicy,
};
use ourstick_core::share::{keep_suitcases, publish_folder, Budget, Publication};
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, IrohPublisher, Network};
use ourstick_core::transport::Publisher;
use ourstick_core::{
    CipherId, ManifestHint, MemoryStore, OwnedStick, StickKey, StickSigningKey, Store, Ticket,
};
use rand::RngCore;
use tempfile::TempDir;

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
        Self::with(network, files, None).await
    }

    /// Like [`Owner::new`], with no suitcases' keeper: nothing is served
    /// but the manifest, unless the test puts it in a suitcase itself.
    pub async fn bare(files: &[(&str, &[u8])]) -> Self {
        Self::build(Network::LocalOnly, files, None).await
    }

    /// Keeps the suitcases of `publisher` (a restarted owner's) for this
    /// owner's stick, in the background.
    pub fn keep(&self, publisher: &IrohPublisher) {
        tokio::spawn(keep_suitcases(
            publisher.clone(),
            self.store.clone(),
            self.signing_key.stick_id(),
            self.stick_key.clone(),
            Budget::new(1 << 33),
        ));
    }

    /// A drive of `size` bytes (the app's stick), marked as such.
    pub async fn drive(size: u64, files: &[(&str, &[u8])]) -> Self {
        let owner = Self::with(Network::LocalOnly, files, Some(size)).await;
        ourstick_core::share::mark_drive(owner.folder.path(), &owner.signing_key.stick_id())
            .unwrap();
        owner
    }

    async fn with(network: Network, files: &[(&str, &[u8])], drive_size: Option<u64>) -> Self {
        let owner = Self::build(network, files, drive_size).await;
        owner.keep(&owner.publisher);
        owner
    }

    async fn build(network: Network, files: &[(&str, &[u8])], drive_size: Option<u64>) -> Self {
        let folder = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            write_file(folder.path(), path, contents);
        }
        let signing_key = StickSigningKey::from_bytes(&random_key());
        let store = Arc::new(MemoryStore::new());
        store
            .add_owned_stick(&OwnedStick {
                drive_size,
                ..OwnedStick::new(
                    signing_key.stick_id(),
                    "test stick",
                    folder.path(),
                    CipherId::XChaCha20Poly1305,
                    1,
                )
            })
            .unwrap();
        let cache = tempfile::tempdir().unwrap();
        let config = IrohConfig::new(random_key(), network);
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
        self.publish_with(false).await
    }

    /// Publishes, reading every file again if `rescan`.
    pub async fn publish_with(&self, rescan: bool) -> Publication {
        publish_folder(
            &self.publisher,
            self.store.clone(),
            &self.clock,
            &self.signing_key,
            &self.stick_key,
            rescan,
        )
        .await
        .unwrap()
    }

    /// Serves `publication` again, with the blob `extra` allowed too (in no
    /// suitcase: "not ready" until a test puts it in one).
    pub async fn allow_also(&self, publication: &Publication, extra: [u8; 32]) {
        let mut allowed: std::collections::HashSet<_> = publication
            .manifest
            .entries
            .iter()
            .map(|entry| entry.hash)
            .collect();
        allowed.insert(extra);
        let manifest_blob = ourstick_core::share::load_latest_manifest(
            self.publisher.cache_dir(),
            &publication.manifest_hash,
        )
        .unwrap();
        self.publisher
            .publish(publication.record.clone(), manifest_blob, allowed)
            .await
            .unwrap();
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

    /// The blob files of every open suitcase of the owner.
    pub fn suitcase_blob_files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let suitcases = self.cache.path().join("suitcases");
        for suitcase in std::fs::read_dir(suitcases).unwrap() {
            let files = suitcase.unwrap().path().join("files");
            for file in std::fs::read_dir(files).unwrap() {
                found.push(file.unwrap().path());
            }
        }
        found
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

/// Fetches `hash` over `connection`, asking again while the owner's keeper
/// puts it in a suitcase ("not ready").
pub async fn fetch_when_ready(
    connection: &impl ourstick_core::transport::PeerConnection,
    hash: &[u8; 32],
    on_progress: &mut (dyn FnMut(u64) -> ourstick_core::transport::Pace + Send),
) -> Result<ourstick_core::transport::FetchStats, ourstick_core::transport::TransportError> {
    for _ in 0..300 {
        match connection.fetch_blob(hash, on_progress).await {
            Err(ourstick_core::transport::TransportError::NotReady) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            other => return other,
        }
    }
    connection.fetch_blob(hash, on_progress).await
}
