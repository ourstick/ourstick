//! The owner's side on iroh: an encrypted blob cache (iroh-blobs `FsStore`) that
//! serves only the current manifest's blobs, plus the record protocol.
//! See ADR 0005.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::api::blobs::{AddPathOptions, BlobStatus, ImportMode};
use iroh_blobs::api::TempTag;
use iroh_blobs::provider::events::{
    AbortReason, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode, RequestUpdate,
};
use iroh_blobs::store::fs::options::Options;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::store::{GcConfig, ProtectCb, ProtectOutcome};
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash};

use super::activity::Activity;
use super::reader_paths::{WatchedBlobs, RELAY_ONLY_AFTER};
use super::record_protocol::{RecordServer, RECORD_ALPN};
use super::{absolute, address_hints, bind, network_report, node_id, IrohConfig};
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::{AddressHint, NodeId};
use crate::transport::{BlobHash, Publisher, TransportError};

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
    /// Set by the first `keep_only_blobs`. Until then every GC run is
    /// aborted, so a restart never deletes the cache before a full pass has
    /// protected again what it still needs (ADR 0007).
    pruned: Arc<AtomicBool>,
    activity: Activity,
    /// Recomputes `activity`; stopped at shutdown.
    reporter: tokio::task::JoinHandle<()>,
}

type Protected = Arc<Mutex<HashMap<Hash, TempTag>>>;

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
        let blobs = BlobsProtocol::new(
            &store,
            Some(only_ready_gets(
                allowed.clone(),
                protected.clone(),
                activity.clone(),
            )),
        );
        let endpoint: Endpoint = bind(config, Vec::new()).await?;
        let router = Router::builder(endpoint)
            .accept(
                iroh_blobs::ALPN,
                WatchedBlobs {
                    blobs,
                    activity: activity.clone(),
                    relay_only_after: RELAY_ONLY_AFTER,
                },
            )
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

/// Every request kind is set explicitly: gets, get-many and observe are
/// intercepted (and refused below unless allowed), push is disabled.
fn serving_event_mask() -> EventMask {
    EventMask {
        // Intercept, so every request is decided below; with the log of each
        // transfer, for the reader activity.
        get: RequestMode::InterceptLog,
        get_many: RequestMode::Intercept,
        push: RequestMode::Disabled,
        observe: ObserveMode::Intercept,
        ..EventMask::DEFAULT
    }
}

/// What the owner answers a request for a blob: what a peer may receive, and
/// whether it is here yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Serve,
    /// Allowed, but not in the cache yet (ADR 0007): "try again later"
    /// (`ERR_LIMIT`), which the reader waits on. Never answered with Serve:
    /// the provider would then fail with `ERR_INTERNAL`, a failure.
    NotReady,
    Refuse,
}

impl Answer {
    fn decide(is_blob: bool, allowed: bool, ready: bool) -> Self {
        match (is_blob && allowed, ready) {
            (false, _) => Self::Refuse,
            (true, true) => Self::Serve,
            (true, false) => Self::NotReady,
        }
    }
}

/// The single place that decides what a peer may receive: a plain get of a raw
/// blob whose hash is currently allowed and which is in the cache. An allowed
/// blob not in the cache yet is "not ready". Everything else is refused,
/// including push, get-many and observe (iroh-blobs 0.103 routes all of them
/// through the `get` mode, so they arrive here as their own messages).
///
/// Accepted transfers are followed, to count readers and bytes sent.
fn only_ready_gets(
    allowed: Arc<RwLock<HashSet<Hash>>>,
    protected: Protected,
    activity: Activity,
) -> EventSender {
    let (sender, mut messages) = EventSender::channel(32, serving_event_mask());
    tokio::spawn(async move {
        while let Some(message) = messages.recv().await {
            match message {
                ProviderMessage::GetRequestReceived(message) => {
                    let request = &message.request;
                    let decided = Answer::decide(
                        request.ranges.is_blob(),
                        read(&allowed).contains(&request.hash),
                        lock(&protected).contains_key(&request.hash),
                    );
                    let permitted = decided == Answer::Serve;
                    let answer = match decided {
                        Answer::Serve => Ok(()),
                        Answer::NotReady => Err(AbortReason::RateLimited),
                        Answer::Refuse => {
                            tracing::debug!("refused a request for a blob that is not served");
                            Err(AbortReason::Permission)
                        }
                    };
                    let connection = message.inner.connection_id;
                    message.tx.send(answer).await.ok();
                    if permitted {
                        let mut updates = message.rx;
                        let activity = activity.clone();
                        tokio::spawn(async move {
                            let mut open = false;
                            let mut offset = 0;
                            while let Ok(Some(update)) = updates.recv().await {
                                match update {
                                    RequestUpdate::Started(_) => {
                                        activity.started(connection);
                                        open = true;
                                        offset = 0;
                                    }
                                    RequestUpdate::Progress(progress) => {
                                        activity.sent(progress.end_offset.saturating_sub(offset));
                                        offset = progress.end_offset;
                                    }
                                    RequestUpdate::Completed(_) | RequestUpdate::Aborted(_) => {
                                        if open {
                                            activity.ended(connection);
                                            open = false;
                                        }
                                    }
                                }
                            }
                            if open {
                                activity.ended(connection);
                            }
                        });
                    }
                }
                ProviderMessage::PushRequestReceived(message) => {
                    tracing::warn!("refused a push request");
                    message.tx.send(Err(AbortReason::Permission)).await.ok();
                }
                ProviderMessage::GetManyRequestReceived(message) => {
                    message.tx.send(Err(AbortReason::Permission)).await.ok();
                }
                ProviderMessage::ObserveRequestReceived(message) => {
                    message.tx.send(Err(AbortReason::Permission)).await.ok();
                }
                // Anything else is a notification we did not ask for, or a
                // request whose reply channel is dropped here, which refuses it.
                _ => {}
            }
        }
    });
    sender
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
        *write(&self.allowed) = allowed;
        *write(&self.record) = Some(record);
        Ok(())
    }

    async fn keep_only_blobs(&self, keep: HashSet<BlobHash>) -> Result<(), TransportError> {
        let allowed = read(&self.allowed).clone();
        // Blobs served now are never dropped; the GC removes the others.
        lock(&self.protected)
            .retain(|hash, _| keep.contains(hash.as_bytes()) || allowed.contains(hash));
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
        Ok(true)
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

    fn activity(&self) -> tokio::sync::watch::Receiver<crate::transport::ReaderActivity> {
        self.activity.subscribe()
    }

    async fn shutdown(self) {
        self.reporter.abort();
        // Shutting down the router also shuts down the store (BlobsProtocol).
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router did not shut down cleanly");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_kind_is_intercepted_or_disabled() {
        let mask = serving_event_mask();
        assert_eq!(mask.get, RequestMode::InterceptLog);
        assert_eq!(mask.get_many, RequestMode::Intercept);
        assert_eq!(mask.push, RequestMode::Disabled);
        assert_eq!(mask.observe, ObserveMode::Intercept);
    }

    #[test]
    fn only_allowed_blobs_in_the_cache_are_served() {
        assert_eq!(Answer::decide(true, true, true), Answer::Serve);
        assert_eq!(Answer::decide(true, true, false), Answer::NotReady);
        // Not allowed, or not a plain blob: refused, in the cache or not.
        for ready in [true, false] {
            assert_eq!(Answer::decide(true, false, ready), Answer::Refuse);
            assert_eq!(Answer::decide(false, true, ready), Answer::Refuse);
        }
    }
}
