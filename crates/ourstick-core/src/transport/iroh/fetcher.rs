//! The reader's side on iroh: per-stick iroh-blobs stores, one per batch of
//! files (kept across runs so downloads resume; `batches`), and connections
//! to the owner.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use iroh::endpoint::{Connection, PathEvent};
use iroh::{Endpoint, EndpointAddr};
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::get::fsm::DecodeError;
use iroh_blobs::get::GetError;
use iroh_blobs::protocol::{ERR_INTERNAL, ERR_LIMIT, ERR_PERMISSION};
use iroh_blobs::{Hash, HashAndFormat};
use tokio::sync::watch;

use super::batches::Batches;
use super::record_protocol::{request_record, RECORD_ALPN};
use super::report_protocol::report_on;
use super::{absolute, bind, is_relay, network_report, owner_addr, IrohConfig};
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::Ticket;
use crate::transport::{
    BlobHash, FetchStats, Fetcher, Pace, PathKind, PathUsage, PeerConnection, TransportError,
};

/// Bytes received per path, keyed by the path id (its `Debug` form: iroh does
/// not export the id type). Open paths are read live, closed ones are kept
/// from their close event, so nothing is counted twice.
type UsageByPath = Arc<Mutex<BTreeMap<String, (Route, u64)>>>;

/// How one path reaches the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Relay,
    DirectIpv4,
    DirectIpv6,
}

fn route_of(addr: &iroh::TransportAddr) -> Route {
    match addr {
        iroh::TransportAddr::Ip(ip) if ip.is_ipv6() => Route::DirectIpv6,
        _ if is_relay(addr) => Route::Relay,
        _ => Route::DirectIpv4,
    }
}

/// A reader's local blob stores for one stick, and the endpoint to reach
/// owners.
pub struct IrohFetcher {
    endpoint: Endpoint,
    batches: Batches,
    relay_description: String,
}

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

fn raw(hash: &BlobHash) -> HashAndFormat {
    HashAndFormat::raw(Hash::from_bytes(*hash))
}

impl IrohFetcher {
    /// Opens (or creates) the stores in `store_dir`. Partial blobs from an
    /// earlier, interrupted run are still there and will be resumed.
    pub async fn start(config: &IrohConfig, store_dir: &Path) -> Result<Self, TransportError> {
        let batches = Batches::open(absolute(store_dir)?).await?;
        let endpoint = bind(config, Vec::new()).await?;
        Ok(Self {
            endpoint,
            batches,
            relay_description: config.relay_description(),
        })
    }
}

impl Fetcher for IrohFetcher {
    type Connection = IrohConnection;

    fn network_report(&self) -> Option<crate::transport::NetworkReport> {
        network_report(&self.endpoint)
    }

    fn relay_description(&self) -> String {
        self.relay_description.clone()
    }

    /// On a connection of its own (one QUIC connection speaks one
    /// protocol), so a report needs no download going on.
    async fn report(&self, ticket: &Ticket, sealed: &[u8]) -> Result<(), TransportError> {
        report_on(&self.endpoint, ticket, sealed).await
    }

    async fn connect(&self, ticket: &Ticket) -> Result<IrohConnection, TransportError> {
        let addr = owner_addr(ticket)?;
        let blobs = self
            .endpoint
            .connect(addr.clone(), iroh_blobs::ALPN)
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let (path_sender, path_receiver) = watch::channel(current_path(&blobs));
        let usage = UsageByPath::default();
        let watcher = tokio::spawn(watch_path(blobs.clone(), path_sender, usage.clone()));
        Ok(IrohConnection {
            endpoint: self.endpoint.clone(),
            batches: self.batches.clone(),
            addr,
            blobs,
            path_receiver,
            watcher,
            usage,
        })
    }

    async fn read_blob(&self, hash: &BlobHash) -> Result<Vec<u8>, TransportError> {
        let bytes = self
            .batches
            .store()
            .await
            .blobs()
            .get_bytes(Hash::from_bytes(*hash))
            .await
            .map_err(storage)?;
        Ok(bytes.to_vec())
    }

    async fn export_blob(&self, hash: &BlobHash, target: &Path) -> Result<(), TransportError> {
        self.batches
            .store()
            .await
            .blobs()
            .export(Hash::from_bytes(*hash), absolute(target)?)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn local_bytes(&self, hash: &BlobHash) -> Result<u64, TransportError> {
        let local = self
            .batches
            .store()
            .await
            .remote()
            .local(raw(hash))
            .await
            .map_err(storage)?;
        Ok(local.local_bytes())
    }

    async fn use_batch(&self, name: &str) -> Result<(), TransportError> {
        self.batches.use_batch(name).await
    }

    async fn drop_batch(&self, name: &str) -> Result<(), TransportError> {
        self.batches.drop_batch(name).await
    }

    async fn keep_only_batches(&self, keep: &[String]) -> Result<(), TransportError> {
        self.batches.keep_only(keep).await
    }

    async fn shutdown(self, keep: bool) -> Result<(), TransportError> {
        self.endpoint.close().await;
        self.batches.shutdown(keep).await
    }
}

fn current_path(connection: &Connection) -> PathKind {
    let paths = connection.paths();
    match paths.iter().find(|path| path.is_selected()) {
        Some(path) if is_relay(path.remote_addr()) => PathKind::Relay,
        Some(_) => PathKind::Direct,
        None => PathKind::Unknown,
    }
}

/// Publishes the selected path every time iroh selects a new one.
async fn watch_path(connection: Connection, sender: watch::Sender<PathKind>, usage: UsageByPath) {
    let mut events = std::pin::pin!(connection.path_events());
    while let Some(event) = events.next().await {
        // Keep the final byte count of every path that closes.
        if let PathEvent::Closed {
            id,
            remote_addr,
            last_stats,
            ..
        } = &event
        {
            usage
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    format!("{id:?}"),
                    (route_of(remote_addr), last_stats.udp_rx.bytes),
                );
        }
        if matches!(event, PathEvent::Selected { .. } | PathEvent::Closed { .. }) {
            let path = current_path(&connection);
            sender.send_if_modified(|current| {
                let changed = *current != path;
                *current = path;
                changed
            });
        }
    }
}

/// A connection from a reader to the owner of a stick.
pub struct IrohConnection {
    endpoint: Endpoint,
    batches: Batches,
    addr: EndpointAddr,
    blobs: Connection,
    path_receiver: watch::Receiver<PathKind>,
    watcher: tokio::task::JoinHandle<()>,
    usage: UsageByPath,
}

fn is_hash_mismatch(error: &GetError) -> bool {
    matches!(
        error,
        GetError::Decode {
            source: DecodeError::LeafHashMismatch { .. } | DecodeError::ParentHashMismatch { .. },
            ..
        }
    )
}

fn get_error(error: GetError) -> TransportError {
    if is_hash_mismatch(&error) {
        TransportError::Corrupt
    } else if error.iroh_error_code() == Some(ERR_PERMISSION) {
        TransportError::Refused
    } else if error.iroh_error_code() == Some(ERR_LIMIT) {
        // "OK to try again later" (iroh-blobs): the owner is preparing the
        // blob (ADR 0007).
        TransportError::NotReady
    } else if error.iroh_error_code() == Some(ERR_INTERNAL) {
        // The owner checks each part against the blob's tree before it sends
        // it, and stops with this code when its file no longer gives that
        // blob (it changed, ADR 0017).
        TransportError::OwnerFailed
    } else {
        TransportError::Interrupted(error.to_string())
    }
}

impl PeerConnection for IrohConnection {
    fn path(&self) -> PathKind {
        *self.path_receiver.borrow()
    }

    fn path_changes(&self) -> watch::Receiver<PathKind> {
        self.path_receiver.clone()
    }

    fn has_direct_path(&self) -> bool {
        self.blobs.paths().iter().any(|path| path.is_ip())
    }

    fn usage(&self) -> PathUsage {
        let mut by_path = self
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for path in self.blobs.paths().iter() {
            by_path.insert(
                format!("{:?}", path.id()),
                (route_of(path.remote_addr()), path.stats().udp_rx.bytes),
            );
        }
        let mut usage = PathUsage::default();
        for (route, bytes) in by_path.values() {
            match route {
                Route::Relay => usage.relay += bytes,
                Route::DirectIpv4 => usage.direct += bytes,
                Route::DirectIpv6 => {
                    usage.direct += bytes;
                    usage.direct_ipv6 += bytes;
                }
            }
        }
        usage
    }

    async fn fetch_record(
        &self,
        stick_id: &StickId,
    ) -> Result<SignedManifestRecord, TransportError> {
        let connection = self
            .endpoint
            .connect(self.addr.clone(), RECORD_ALPN)
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let record = request_record(&connection, stick_id).await;
        connection.close(0u32.into(), b"done");
        record
    }

    async fn fetch_blob(
        &self,
        hash: &BlobHash,
        on_progress: &mut (dyn FnMut(u64) -> Pace + Send),
    ) -> Result<FetchStats, TransportError> {
        let local = self
            .batches
            .store()
            .await
            .remote()
            .local(raw(hash))
            .await
            .map_err(storage)?;
        let already_local = local.local_bytes();
        if local.is_complete() {
            return Ok(FetchStats {
                already_local,
                received: 0,
            });
        }
        let store = self.batches.store().await;
        let mut progress =
            std::pin::pin!(store.remote().fetch(self.blobs.clone(), raw(hash)).stream());
        while let Some(item) = progress.next().await {
            match item {
                GetProgressItem::Progress(received) => match on_progress(received) {
                    ControlFlow::Break(()) => {
                        // Closing the connection stops the transfer mid-way;
                        // what was verified so far stays in the store.
                        self.blobs.close(0u32.into(), b"cancelled");
                        return Err(TransportError::Cancelled);
                    }
                    // The fetch runs inside this stream: while it is not
                    // polled nothing is read, and QUIC flow control slows
                    // the sender.
                    ControlFlow::Continue(wait) if !wait.is_zero() => {
                        tokio::time::sleep(wait).await;
                    }
                    ControlFlow::Continue(_) => {}
                },
                GetProgressItem::Done(stats) => {
                    return Ok(FetchStats {
                        already_local,
                        received: stats.payload_bytes_read,
                    });
                }
                GetProgressItem::Error(error) => return Err(get_error(error)),
            }
        }
        Err(TransportError::Interrupted(
            "the transfer ended early".into(),
        ))
    }

    async fn close(self) {
        self.watcher.abort();
        self.blobs.close(0u32.into(), b"bye");
    }
}
