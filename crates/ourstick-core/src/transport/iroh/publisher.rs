//! The owner's side on iroh: blobs served from suitcases (ADR 0007), the
//! current manifest from memory, through our own handler
//! ([`super::handler`]), plus the record protocol.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;

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

/// Serves one stick; its suitcases live under `cache_dir`. Cheap to clone:
/// clones share everything (the suitcases' keeper holds one), and
/// [`Publisher::shutdown`] on any of them stops them all.
#[derive(Clone)]
pub struct IrohPublisher {
    router: Router,
    cache_dir: PathBuf,
    record: Arc<RwLock<Option<SignedManifestRecord>>>,
    /// What is served, decided in one place.
    serving: Serving,
    suitcases: Arc<Suitcases>,
    /// The store serving the current manifest.
    manifest: Arc<Mutex<Option<(StoreId, MemStore)>>>,
    /// What readers ask for, until the owner's policy takes it.
    demand: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Demand>>>>,
    activity: Activity,
    /// Recomputes `activity`; stopped at shutdown.
    reporter: tokio::task::AbortHandle,
}

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl IrohPublisher {
    /// Starts serving `stick_id`, with its suitcases in `cache_dir`
    /// (whatever an earlier run left there is removed). Nothing is served
    /// until the first [`Publisher::publish`].
    pub async fn start(
        config: &IrohConfig,
        stick_id: StickId,
        cache_dir: &Path,
    ) -> Result<Self, TransportError> {
        let cache_dir = absolute(cache_dir)?;
        tokio::fs::create_dir_all(&cache_dir).await?;
        let record = Arc::new(RwLock::new(None));
        let activity = Activity::new();
        let reporter = tokio::spawn(activity.clone().report()).abort_handle();
        let serving = Serving::default();
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
            cache_dir,
            record,
            serving,
            suitcases: Arc::new(suitcases),
            manifest: Arc::new(Mutex::new(None)),
            demand: Arc::new(Mutex::new(Some(demand))),
            activity,
            reporter,
        })
    }

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

    async fn publish(
        &self,
        record: SignedManifestRecord,
        manifest_blob: Vec<u8>,
        allowed: HashSet<BlobHash>,
    ) -> Result<(), TransportError> {
        let manifest = Hash::from_bytes(record.unverified_record().manifest_hash);
        if Hash::new(&manifest_blob) != manifest {
            return Err(storage("the manifest blob is not the record's"));
        }
        // The manifest from memory, in a store of its own.
        let store = MemStore::new();
        store.add_bytes(manifest_blob).await.map_err(storage)?;
        let id = self.suitcases.next_id();
        self.serving.add_store(id, (*store).clone());
        self.serving.set_ready(id, manifest);
        let mut allowed: HashSet<Hash> = allowed.into_iter().map(Hash::from_bytes).collect();
        allowed.insert(manifest);
        self.serving.allow(allowed);
        *self
            .record
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(record);
        let previous = lock(&self.manifest).replace((id, store));
        // The last manifest leaves once no transfer holds it.
        if let Some((previous, old)) = previous {
            let serving = self.serving.clone();
            tokio::spawn(async move {
                serving.remove_store(previous).await;
                old.shutdown().await.ok();
            });
        }
        Ok(())
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
        lock(&self.demand).take()
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
        let manifest = lock(&self.manifest).take();
        if let Some((_, store)) = manifest {
            store.shutdown().await.ok();
        }
    }
}
