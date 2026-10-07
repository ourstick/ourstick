//! The owner's side on iroh: blobs made from the files when asked (ADR 0017), the
//! current manifest from memory, through our own handler
//! ([`super::handler`]), plus the record protocol.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;

use super::activity::Activity;
use super::handler::BlobsHandler;
use super::reader_paths::RELAY_ONLY_AFTER;
use super::record_protocol::{RecordServer, RECORD_ALPN};
use super::report_protocol::{ReportServer, REPORT_ALPN, WAITING_REPORTS};
use super::serving::{Serving, StoreId};
use super::{absolute, address_hints, bind, network_report, node_id, IrohConfig};
use crate::manifest::Manifest;
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId};
use crate::transport::{
    BlobHash, Demand, PublishedVersion, Publisher, ServedFiles, TransportError,
};

/// Serves one stick; its cache lives under `cache_dir`. Cheap to clone:
/// clones share everything (the owner's files keeper holds one), and
/// [`Publisher::shutdown`] on any of them stops them all.
#[derive(Clone)]
pub struct IrohPublisher {
    router: Router,
    cache_dir: PathBuf,
    record: Arc<RwLock<Option<SignedManifestRecord>>>,
    /// What is served, decided in one place.
    serving: Serving,
    /// The next store's id (each manifest's, and the files' one).
    next_store: Arc<AtomicU64>,
    /// Transfers read the files through the disks' turns (ADR 0017,
    /// amendment 1).
    reads: super::made_store::Reads,
    /// The store serving the current manifest.
    manifest: Arc<Mutex<Option<(StoreId, MemStore)>>>,
    /// The owner's own store (ADR 0017), once it serves files.
    made: Arc<Mutex<Option<(StoreId, iroh_blobs::api::Store, super::made_store::Served)>>>,
    /// What readers ask for, until the owner's policy takes it.
    demand: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Demand>>>>,
    /// Where the owner's own store tells what it could not serve: weak, so
    /// the demand ends (and the owner's keeper with it) once the handler,
    /// which holds the strong one, is gone with the router.
    demand_sender: tokio::sync::mpsc::WeakUnboundedSender<Demand>,
    /// The version published last, for the owner's files keeper.
    published: Arc<tokio::sync::watch::Sender<Option<PublishedVersion>>>,
    /// Readers' sealed reports (ADR 0015), until the owner's side takes them.
    reports: Arc<Mutex<Option<tokio::sync::mpsc::Receiver<Vec<u8>>>>>,
    activity: Activity,
    /// Recomputes `activity`; stopped at shutdown.
    reporter: tokio::task::AbortHandle,
}

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

/// The threads that make blobs for transfers, for the whole program (ADR
/// 0017, rule 8): they read files and encrypt, which blocks, so they are not
/// the program's own runtime's.
fn made_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            // For the probes before transfers (`serving::Probe`).
            .max_blocking_threads(4)
            .thread_keep_alive(std::time::Duration::from_secs(5 * 60))
            .thread_name("made-blobs")
            .enable_all()
            .build()
            .expect("a runtime for made blobs")
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl IrohPublisher {
    /// Starts serving `stick_id`, with its cache in `cache_dir` (the latest
    /// manifest and the outboards; the suitcases an older version kept
    /// there are removed, ADR 0017). Nothing is served until the first
    /// [`Publisher::publish`].
    pub async fn start(
        config: &IrohConfig,
        stick_id: StickId,
        cache_dir: &Path,
        turns: Arc<crate::disk::turns::DiskTurns>,
    ) -> Result<Self, TransportError> {
        let cache_dir = absolute(cache_dir)?;
        tokio::fs::create_dir_all(&cache_dir).await?;
        let record = Arc::new(RwLock::new(None));
        let activity = Activity::new();
        let reads = super::made_store::Reads::new(turns);
        let giving_way = {
            let reads = reads.clone();
            Arc::new(move || reads.giving_way())
        };
        let reporter = tokio::spawn(activity.clone().report(giving_way)).abort_handle();
        let serving = Serving::default();
        // In the background: an old cache may be big, and nothing uses it.
        let suitcases = cache_dir.join("suitcases");
        tokio::spawn(async move {
            match tokio::fs::remove_dir_all(&suitcases).await {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    tracing::warn!(%error, "the old suitcases could not be removed");
                }
                _ => {}
            }
        });
        let (demand_sender, demand) = tokio::sync::mpsc::unbounded_channel();
        let (report_sender, reports) = tokio::sync::mpsc::channel(WAITING_REPORTS);
        let handler = BlobsHandler::new(
            serving.clone(),
            activity.clone(),
            RELAY_ONLY_AFTER,
            demand_sender.clone(),
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
            .accept(
                REPORT_ALPN,
                ReportServer {
                    stick_id,
                    reports: report_sender,
                },
            )
            .spawn();
        Ok(Self {
            router,
            cache_dir,
            record,
            serving,
            next_store: Arc::new(AtomicU64::new(1)),
            reads,
            manifest: Arc::new(Mutex::new(None)),
            made: Arc::new(Mutex::new(None)),
            demand: Arc::new(Mutex::new(Some(demand))),
            demand_sender: demand_sender.downgrade(),
            published: Arc::new(tokio::sync::watch::Sender::new(None)),
            reports: Arc::new(Mutex::new(Some(reports))),
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
        decoded: Arc<Manifest>,
    ) -> Result<(), TransportError> {
        let manifest_hash = record.unverified_record().manifest_hash;
        let manifest = Hash::from_bytes(manifest_hash);
        if Hash::new(&manifest_blob) != manifest {
            return Err(storage("the manifest blob is not the record's"));
        }
        // The manifest from memory, in a store of its own.
        let store = MemStore::new();
        store.add_bytes(manifest_blob).await.map_err(storage)?;
        let id = self.next_store.fetch_add(1, Ordering::Relaxed);
        self.serving.add_store(id, (*store).clone());
        self.serving.set_ready(id, manifest);
        let mut allowed: HashSet<Hash> = decoded
            .entries
            .iter()
            .map(|entry| Hash::from_bytes(entry.hash))
            .collect();
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
        crate::transport::hand_over(&self.published, manifest_hash, decoded);
        Ok(())
    }

    fn serve_files(&self, cipher: Arc<dyn crate::cipher::Cipher>, files: ServedFiles) {
        let mut made = lock(&self.made);
        let (id, _, served) = made.get_or_insert_with(|| {
            let outboards = crate::manifest::hashing::outboards_dir(&self.cache_dir);
            let id = self.next_store.fetch_add(1, Ordering::Relaxed);
            // What it cannot serve is not ready from then on, and the owner
            // is told (ADR 0017, rule 4).
            let serving = self.serving.clone();
            let demand = self.demand_sender.clone();
            let on_not_served: super::made_store::OnNotServed = Arc::new(move |hash, changed| {
                serving.remove_ready(id, &hash);
                if let Some(demand) = demand.upgrade() {
                    demand
                        .send(Demand::NotServed {
                            hash: *hash.as_bytes(),
                            changed,
                        })
                        .ok();
                }
            });
            let (store, served) = super::made_store::start(
                cipher.clone(),
                outboards,
                made_runtime().handle().clone(),
                on_not_served,
                Some(self.reads.clone()),
            );
            self.serving.add_store(id, store.clone());
            self.serving.set_probe(
                id,
                super::made_store::probe(served.clone(), made_runtime().handle().clone()),
            );
            (id, store, served)
        });
        served.set(cipher, files, |files| {
            let ready = files
                .files
                .keys()
                .map(|hash| Hash::from_bytes(*hash))
                .collect();
            self.serving.replace_ready(*id, ready);
        });
    }

    fn serve_more(
        &self,
        manifest: &Arc<crate::manifest::Manifest>,
        files: Vec<(BlobHash, crate::transport::ServedFile)>,
    ) {
        let made = lock(&self.made);
        let Some((id, _, served)) = made.as_ref() else {
            return;
        };
        served.add(manifest, files, |hash| self.serving.set_ready(*id, hash));
    }

    fn published(&self) -> tokio::sync::watch::Receiver<Option<PublishedVersion>> {
        self.published.subscribe()
    }

    fn readers_waiting(&self, window: std::time::Duration) -> bool {
        self.serving.not_ready_within(window)
    }

    fn take_demand(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<Demand>> {
        lock(&self.demand).take()
    }

    fn take_reports(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<u8>>> {
        lock(&self.reports).take()
    }

    fn activity(&self) -> tokio::sync::watch::Receiver<crate::transport::ReaderActivity> {
        self.activity.subscribe()
    }

    async fn shutdown(self) {
        self.reporter.abort();
        // First: a transfer waiting for a disk's turn would keep the router
        // from shutting down.
        self.reads.stop();
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router did not shut down cleanly");
        }
        let manifest = lock(&self.manifest).take();
        if let Some((_, store)) = manifest {
            store.shutdown().await.ok();
        }
        let made = lock(&self.made).take();
        if let Some((_, store, _)) = made {
            store.shutdown().await.ok();
        }
    }
}
