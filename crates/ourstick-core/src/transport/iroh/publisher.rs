//! The owner's side on iroh: an encrypted blob cache (iroh-blobs `FsStore`) that
//! serves only the current manifest's blobs, through our own handler
//! ([`super::handler`]), plus the record protocol. See ADR 0005 and ADR 0007.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::api::blobs::{AddPathOptions, BlobStatus, ImportMode};
use iroh_blobs::api::TempTag;
use iroh_blobs::store::fs::options::Options;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::store::{GcConfig, ProtectCb, ProtectOutcome};
use iroh_blobs::{BlobFormat, Hash};

use super::activity::Activity;
use super::handler::BlobsHandler;
use super::reader_paths::RELAY_ONLY_AFTER;
use super::record_protocol::{RecordServer, RECORD_ALPN};
use super::serving::{Serving, StoreId};
use super::suitcases::Suitcases;
use super::{absolute, address_hints, bind, network_report, node_id, IrohConfig};
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId};
use crate::transport::{BlobHash, Demand, Publisher, SuitcaseId, TransportError};

/// Serves one stick from an encrypted blob cache in `cache_dir`.
pub struct IrohPublisher {
    router: Router,
    store: FsStore,
    cache_dir: PathBuf,
    staging_dir: PathBuf,
    record: Arc<RwLock<Option<SignedManifestRecord>>>,
    allowed: Arc<RwLock<HashSet<Hash>>>,
    /// A temp tag for every blob that must stay: every blob imported or kept
    /// since the last [`Publisher::keep_only_blobs`] that did not drop it. Held
    /// from import on, so there is no moment when a wanted blob is unprotected
    /// (iroh-blobs #247). Also what "ready" means: an allowed blob is served
    /// only once it is in here.
    protected: Protected,
    /// What is served, decided in one place; the cache is its store
    /// [`CACHE`].
    serving: Serving,
    suitcases: Suitcases,
    /// What readers ask for, until the owner's policy takes it.
    demand: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Demand>>>,
    /// Set by the first `keep_only_blobs`. Until then every GC run is
    /// aborted, so a restart never deletes the cache before a full pass has
    /// protected again what it still needs (ADR 0007).
    pruned: Arc<AtomicBool>,
    activity: Activity,
    /// Recomputes `activity`; stopped at shutdown.
    reporter: tokio::task::JoinHandle<()>,
}

type Protected = Arc<Mutex<HashMap<Hash, TempTag>>>;

/// The cache's id among the stores that serve.
const CACHE: StoreId = 0;

fn lock(protected: &Protected) -> std::sync::MutexGuard<'_, HashMap<Hash, TempTag>> {
    protected
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

impl IrohPublisher {
    /// Opens (or creates) the cache in `cache_dir` and starts serving `stick_id`.
    /// Nothing is served until the first [`Publisher::publish`], and nothing
    /// is removed from the cache until the first
    /// [`Publisher::keep_only_blobs`].
    pub async fn start(
        config: &IrohConfig,
        stick_id: StickId,
        cache_dir: &Path,
    ) -> Result<Self, TransportError> {
        let cache_dir = absolute(cache_dir)?;
        let store_dir = cache_dir.join("blobs");
        let staging_dir = cache_dir.join("staging");
        tokio::fs::create_dir_all(&store_dir).await?;
        tokio::fs::create_dir_all(&staging_dir).await?;

        let allowed = Arc::new(RwLock::new(HashSet::new()));
        let pruned = Arc::new(AtomicBool::new(false));
        let protected: Protected = Arc::new(Mutex::new(HashMap::new()));
        let protect: ProtectCb = {
            let allowed = allowed.clone();
            let pruned = pruned.clone();
            Arc::new(move |live: &mut HashSet<Hash>| {
                let outcome = if pruned.load(Ordering::SeqCst) {
                    // Belt and braces next to the temp tags: the served set.
                    live.extend(read(&allowed).iter().copied());
                    ProtectOutcome::Continue
                } else {
                    ProtectOutcome::Abort
                };
                Box::pin(std::future::ready(outcome))
            })
        };
        let mut options = Options::new(&store_dir);
        options.gc = Some(GcConfig {
            interval: config.gc_interval,
            add_protected: Some(protect),
        });
        let store = FsStore::load_with_opts(store_dir.join("blobs.db"), options)
            .await
            .map_err(storage)?;

        let record = Arc::new(RwLock::new(None));
        let activity = Activity::new();
        let reporter = tokio::spawn(activity.clone().report());
        let serving = Serving::default();
        serving.add_store(CACHE, (*store).clone());
        let suitcases = Suitcases::start(cache_dir.join("suitcases"), serving.clone()).await?;
        let (demand_sender, demand) = tokio::sync::mpsc::unbounded_channel();
        let handler = BlobsHandler::new(
            serving.clone(),
            activity.clone(),
            RELAY_ONLY_AFTER,
            demand_sender,
        );
        let endpoint: Endpoint = bind(config, Vec::new()).await?;
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, handler)
            .accept(
                RECORD_ALPN,
                RecordServer {
                    stick_id,
                    record: record.clone(),
                },
            )
            .spawn();

        Ok(Self {
            router,
            store,
            cache_dir,
            staging_dir,
            record,
            allowed,
            protected,
            serving,
            suitcases,
            demand: Mutex::new(Some(demand)),
            pruned,
            activity,
            reporter,
        })
    }
}

impl IrohPublisher {
    /// Waits until the node is connected to its home relay, so the address
    /// hints include it. Returns false on timeout (the ticket then has direct
    /// addresses only). Do not call with [`Network::LocalOnly`](super::Network):
    /// there is no relay to wait for.
    pub async fn online(&self, timeout: std::time::Duration) -> bool {
        tokio::time::timeout(timeout, self.router.endpoint().online())
            .await
            .is_ok()
    }

    /// The network report once measured, waiting up to `timeout` for it;
    /// `None` if the measurement has not finished by then.
    pub async fn wait_network_report(
        &self,
        timeout: std::time::Duration,
    ) -> Option<crate::transport::NetworkReport> {
        super::wait_network_report(self.router.endpoint(), timeout).await
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Publisher for IrohPublisher {
    fn node_id(&self) -> NodeId {
        node_id(self.router.endpoint())
    }

    fn address_hints(&self) -> Vec<AddressHint> {
        address_hints(self.router.endpoint())
    }

    fn network_report(&self) -> Option<crate::transport::NetworkReport> {
        network_report(self.router.endpoint())
    }

    fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    async fn import_blob(&self, blob_file: &Path) -> Result<BlobHash, TransportError> {
        let options = AddPathOptions {
            path: absolute(blob_file)?,
            format: BlobFormat::Raw,
            mode: ImportMode::Copy,
        };
        let tag = self
            .store
            .blobs()
            .add_path_with_opts(options)
            .temp_tag()
            .await
            .map_err(storage)?;
        let hash = tag.hash();
        lock(&self.protected).insert(hash, tag);
        self.serving.set_ready(CACHE, hash);
        Ok(*hash.as_bytes())
    }

    async fn publish(
        &self,
        record: SignedManifestRecord,
        allowed: HashSet<BlobHash>,
    ) -> Result<(), TransportError> {
        let manifest = Hash::from_bytes(record.unverified_record().manifest_hash);
        // The manifest must be ready: a reader cannot wait on it.
        if !lock(&self.protected).contains_key(&manifest) {
            return Err(TransportError::Storage(format!(
                "manifest blob {} is not in the cache",
                manifest.fmt_short()
            )));
        }
        let mut allowed: HashSet<Hash> = allowed.into_iter().map(Hash::from_bytes).collect();
        allowed.insert(manifest);
        self.serving.allow(allowed.clone());
        *write(&self.allowed) = allowed;
        *write(&self.record) = Some(record);
        Ok(())
    }

    async fn keep_only_blobs(&self, keep: HashSet<BlobHash>) -> Result<(), TransportError> {
        let allowed = read(&self.allowed).clone();
        // Blobs served now are never dropped; the GC removes the others.
        let kept = |hash: &Hash| keep.contains(hash.as_bytes()) || allowed.contains(hash);
        lock(&self.protected).retain(|hash, _| kept(hash));
        self.serving.retain_ready(CACHE, kept);
        self.pruned.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn keep_blob(&self, hash: &BlobHash) -> Result<bool, TransportError> {
        let hash = Hash::from_bytes(*hash);
        // Protect first, then check: the GC cannot slip in between (#247).
        let tag = self.store.tags().temp_tag(hash).await.map_err(storage)?;
        if !matches!(
            self.store.blobs().status(hash).await.map_err(storage)?,
            BlobStatus::Complete { .. }
        ) {
            return Ok(false);
        }
        lock(&self.protected).insert(hash, tag);
        self.serving.set_ready(CACHE, hash);
        Ok(true)
    }

    async fn read_blob(&self, hash: &BlobHash) -> Result<Option<Vec<u8>>, TransportError> {
        if !self.has_blob(hash).await? {
            return Ok(None);
        }
        let bytes = self
            .store
            .blobs()
            .get_bytes(Hash::from_bytes(*hash))
            .await
            .map_err(storage)?;
        Ok(Some(bytes.to_vec()))
    }

    async fn has_blob(&self, hash: &BlobHash) -> Result<bool, TransportError> {
        let status = self
            .store
            .blobs()
            .status(Hash::from_bytes(*hash))
            .await
            .map_err(storage)?;
        Ok(matches!(status, BlobStatus::Complete { .. }))
    }

    async fn open_suitcase(&self) -> Result<(SuitcaseId, PathBuf), TransportError> {
        self.suitcases.open().await
    }

    async fn add_to_suitcase(
        &self,
        suitcase: SuitcaseId,
        blob_file: &Path,
        expected: &BlobHash,
    ) -> Result<bool, TransportError> {
        self.suitcases.add(suitcase, blob_file, expected).await
    }

    async fn evict(&self, suitcase: SuitcaseId) -> Result<(), TransportError> {
        self.suitcases.evict(suitcase).await
    }

    fn take_demand(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<Demand>> {
        self.demand
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn activity(&self) -> tokio::sync::watch::Receiver<crate::transport::ReaderActivity> {
        self.activity.subscribe()
    }

    async fn shutdown(self) {
        self.reporter.abort();
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router did not shut down cleanly");
        }
        self.suitcases.close_all().await;
        // Our handler does not own the store: closed here, after the router.
        if let Err(error) = self.store.shutdown().await {
            tracing::warn!(%error, "the cache did not shut down cleanly");
        }
    }
}
