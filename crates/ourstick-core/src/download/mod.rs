//! The reader's side: fetch and check the latest record, fetch and decrypt the
//! manifest, then fetch, decrypt and write every file. Interrupted downloads
//! resume, because the fetcher keeps partial blobs.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod entry;
mod files;
mod path;
mod relay;

use entry::download_file;
use files::{check_manifest_paths, prepare_partial_dir};
use path::ReceivingPath;
use relay::{settle, RelayState};

pub use relay::{RelayConsent, RelayPolicy, RelayQuestion};

/// How many times a download reconnects (budget used up, direct path lost)
/// before giving up.
const MAX_RECONNECTS: u32 = 3;

use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, CipherError, CipherId};
use crate::clock::Clock;
use crate::manifest::{Manifest, ManifestError};
use crate::record::{ManifestHint, RecordCheck, RecordError};
use crate::store::{JoinedStick, Store, StoreError};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PathKind, PathUsage, PeerConnection, TransportError};

/// Where partially written files live inside the output folder, so the final
/// rename stays on one file system. Removed at the end.
const PARTIAL_DIR: &str = ".ourstick-partial";

/// Progress of a download, for display. Return [`ControlFlow::Break`] from the
/// callback to cancel.
#[derive(Debug, Clone, Copy)]
pub enum DownloadEvent<'a> {
    Connected {
        path: PathKind,
    },
    /// The data now mostly arrives over another kind of path.
    PathChanged {
        path: PathKind,
    },
    /// No direct path yet: waiting this long for one (ADR 0006). Before the
    /// start (`budget_used` false), or once the relay budget is used up.
    WaitingForDirect {
        seconds: u64,
        budget_used: bool,
    },
    /// This many bytes came through the relay, the whole budget; the download
    /// reconnects and looks for a direct path before going on.
    RelayBudgetUsed {
        bytes: u64,
    },
    /// After the budget was used up, the direct path was lost; the download
    /// reconnects and tries for a direct path again.
    DirectLost,
    Manifest {
        seq: u64,
        files: usize,
        bytes: u64,
        unchanged: bool,
    },
    FileStarted {
        path: &'a str,
        size: u64,
        /// Blob bytes already stored locally from an earlier, interrupted run.
        already_local: u64,
    },
    Progress {
        path: &'a str,
        received: u64,
    },
    FileDone {
        path: &'a str,
        already_local: u64,
        received: u64,
    },
}

/// What a finished download did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadReport {
    pub seq: u64,
    pub files: usize,
    pub bytes: u64,
    /// Blob bytes received over the network (less than the total after a resume).
    pub received: u64,
    /// Bytes that arrived over direct and over relay paths, over all
    /// connections of this download.
    pub usage: PathUsage,
}

/// Why a download failed. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("the download was cancelled; run the same command again to resume")]
    Cancelled,
    #[error("the owner's manifest record was rejected: {0}")]
    Record(#[from] RecordError),
    #[error("{path}: expected {expected} bytes, got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("no direct connection to the owner could be made and using the relay was declined ({remaining} bytes left); run again later, or allow the relay")]
    RelayDeclined { remaining: u64 },
    #[error("the direct connection to the owner kept dropping; run again to resume")]
    DirectLost,
    #[error("the free amount of data through the relay is used up; run again to resume")]
    RelayBudgetUsed,
    #[error("refusing to write {path}: {reason}")]
    UnsafePath { path: String, reason: &'static str },
    #[error("{first} and {second} would be the same file on a disk that ignores letter case, or one is a file and the other needs it as a folder")]
    PathCollision { first: String, second: String },
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Cipher(#[from] CipherError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> DownloadError + '_ {
    move |source| DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn cancelled(error: TransportError) -> DownloadError {
    match error {
        TransportError::Cancelled => DownloadError::Cancelled,
        other => DownloadError::Transport(other),
    }
}

/// Downloads the stick named in `ticket` into `output_dir`.
///
/// Records the stick as joined on first use, and the accepted record in
/// `store`. On any error nothing half-written is left under a final name; run
/// again to resume. The caller shuts the fetcher down afterwards (keeping its
/// store on error, so the next run resumes).
/// Whether file data may go through the relay follows `policy` (ADR 0006);
/// `consent` is asked when a larger transfer has no direct path.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    ticket: &Ticket,
    output_dir: &Path,
    policy: &RelayPolicy,
    consent: &impl RelayConsent,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    let stick_id = ticket.stick_id;
    {
        let store = store.clone();
        let output_dir = output_dir.to_path_buf();
        let now = clock.now_ms();
        run_blocking(move || match store.joined_stick(&stick_id)? {
            Some(joined) => Ok(joined),
            None => {
                let joined = JoinedStick::new(stick_id, output_dir, now);
                store.add_joined_stick(&joined)?;
                Ok::<_, StoreError>(joined)
            }
        })
        .await?;
    }

    let mut relay = RelayState::default();
    let mut reconnects = 0;
    let mut total = PathUsage::default();
    loop {
        // Read again on every attempt: an earlier attempt may have accepted
        // the record, which makes it "unchanged" now.
        let joined = {
            let store = store.clone();
            run_blocking(move || store.joined_stick(&stick_id))
                .await?
                .ok_or(StoreError::StickNotFound)?
        };
        let connection = fetcher.connect(ticket).await?;
        let result = download_over(
            fetcher,
            &connection,
            store.clone(),
            ticket,
            &joined,
            output_dir,
            policy,
            consent,
            &mut relay,
            on_event,
        )
        .await;
        let usage = connection.usage();
        total.direct += usage.direct;
        total.relay += usage.relay;
        total.direct_ipv6 += usage.direct_ipv6;
        relay.relay_used = total.relay;
        connection.close().await;
        let event = match result {
            Ok(report) => {
                return Ok(DownloadReport {
                    usage: total,
                    ..report
                })
            }
            Err(DownloadError::RelayBudgetUsed) if reconnects < MAX_RECONNECTS => {
                relay.budget_spent = true;
                DownloadEvent::RelayBudgetUsed {
                    bytes: relay.relay_used,
                }
            }
            Err(DownloadError::DirectLost) if reconnects < MAX_RECONNECTS => {
                relay.direct_was_lost = true;
                DownloadEvent::DirectLost
            }
            Err(error) => return Err(error),
        };
        reconnects += 1;
        if on_event(event).is_break() {
            return Err(DownloadError::Cancelled);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_over(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    joined: &JoinedStick,
    output_dir: &Path,
    policy: &RelayPolicy,
    consent: &impl RelayConsent,
    relay: &mut RelayState,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    // What the user was told. The tracker below starts from this, not from
    // the path at that later moment: hole punching often succeeds while the
    // manifest is fetched, and a change nobody reported would never be shown.
    let reported_path = connection.path();
    let connected_usage = connection.usage();
    if on_event(DownloadEvent::Connected {
        path: reported_path,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    let signed = connection.fetch_record(&ticket.stick_id).await?;
    let current = joined.current_manifest.map(|current| ManifestHint {
        seq: current.seq,
        hash: current.hash,
    });
    let check = signed.check(&ticket.stick_id, &ticket.manifest_hint, current.as_ref())?;
    let (record, unchanged) = match check {
        RecordCheck::Newer(record) => (record, false),
        RecordCheck::Unchanged(record) => (record, true),
    };

    connection
        .fetch_blob(&record.manifest_hash, &mut |_| ControlFlow::Continue(()))
        .await
        .map_err(cancelled)?;
    let manifest_blob = fetcher.read_blob(&record.manifest_hash).await?;
    let cipher = cipher_of(&manifest_blob, ticket)?;
    let manifest = {
        let cipher = cipher.clone();
        run_blocking(move || -> Result<Manifest, DownloadError> {
            let mut plaintext = Vec::new();
            decrypt_blob(
                cipher.as_ref(),
                &mut manifest_blob.as_slice(),
                &mut plaintext,
            )?;
            Ok(Manifest::from_bytes(&plaintext)?)
        })
        .await?
    };
    // Before anything is accepted or written: every path must be safe here.
    check_manifest_paths(&manifest)?;
    if !unchanged {
        // Accepted only once the manifest it names has been read successfully.
        let store = store.clone();
        run_blocking(move || store.accept_record(&record)).await?;
    }
    if on_event(DownloadEvent::Manifest {
        seq: record.seq,
        files: manifest.entries.len(),
        bytes: manifest.total_size(),
        unchanged,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    // ADR 0006: a short wait for a direct path, then a relay budget, then only
    // a direct path unless the user agrees. The record and manifest above may
    // use the relay.
    let mut remaining = 0;
    for entry in &manifest.entries {
        remaining += entry
            .size
            .saturating_sub(fetcher.local_bytes(&entry.hash).await?);
    }
    let watch = settle(
        fetcher, connection, ticket, policy, consent, relay, remaining, on_event,
    )
    .await?;

    let partial = {
        let output_dir = output_dir.to_path_buf();
        run_blocking(move || prepare_partial_dir(&output_dir)).await?
    };
    let mut received = 0;
    let mut receiving = ReceivingPath::new(reported_path, connected_usage);
    for entry in &manifest.entries {
        received += download_file(
            fetcher,
            connection,
            &cipher,
            entry,
            output_dir,
            &partial,
            &mut receiving,
            watch,
            on_event,
        )
        .await?;
    }
    tokio::fs::remove_dir_all(&partial)
        .await
        .map_err(io_error(&partial))?;

    Ok(DownloadReport {
        seq: record.seq,
        files: manifest.entries.len(),
        bytes: manifest.total_size(),
        received,
        usage: PathUsage::default(),
    })
}

/// The stick's cipher, read from the manifest blob header. Every file blob must
/// use the same one (`decrypt_blob` checks).
fn cipher_of(manifest_blob: &[u8], ticket: &Ticket) -> Result<Arc<dyn Cipher>, DownloadError> {
    let byte = *manifest_blob.get(1).ok_or(CipherError::Truncated)?;
    Ok(cipher_for(CipherId::from_byte(byte)?, &ticket.stick_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_maps_to_its_own_error() {
        assert!(matches!(
            cancelled(TransportError::Cancelled),
            DownloadError::Cancelled
        ));
        assert!(matches!(
            cancelled(TransportError::Corrupt),
            DownloadError::Transport(TransportError::Corrupt)
        ));
    }
}
