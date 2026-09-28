//! The reader's side: fetch and check the latest record, fetch and decrypt the
//! manifest, then fetch, decrypt and write every file. Interrupted downloads
//! resume, because the fetcher keeps partial blobs.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod files;
mod path;

use files::{check_manifest_paths, decrypt_into_place, prepare_partial_dir, target_path};
use path::ReceivingPath;

use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, CipherError, CipherId};
use crate::clock::Clock;
use crate::manifest::{Entry, Manifest, ManifestError};
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
    /// Bytes that arrived over direct and over relay paths.
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
pub async fn download(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    ticket: &Ticket,
    output_dir: &Path,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    let stick_id = ticket.stick_id;
    let joined = {
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
        .await?
    };

    let connection = fetcher.connect(ticket).await?;
    let result = download_over(
        fetcher,
        &connection,
        store,
        ticket,
        &joined,
        output_dir,
        on_event,
    )
    .await
    .map(|report| DownloadReport {
        usage: connection.usage(),
        ..report
    });
    connection.close().await;
    result
}

async fn download_over(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    joined: &JoinedStick,
    output_dir: &Path,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    if on_event(DownloadEvent::Connected {
        path: connection.path(),
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

    let partial = {
        let output_dir = output_dir.to_path_buf();
        run_blocking(move || prepare_partial_dir(&output_dir)).await?
    };
    let mut received = 0;
    let mut receiving = ReceivingPath::new(connection.path(), connection.usage());
    for entry in &manifest.entries {
        received += download_file(
            fetcher,
            connection,
            &cipher,
            entry,
            output_dir,
            &partial,
            &mut receiving,
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

#[allow(clippy::too_many_arguments)]
async fn download_file(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    cipher: &Arc<dyn Cipher>,
    entry: &Entry,
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<u64, DownloadError> {
    let path = entry.path.as_str();
    // Early check, so a bad path fails before the transfer; checked again
    // right before the file is moved into place.
    {
        let output_dir = output_dir.to_path_buf();
        let path = path.to_owned();
        run_blocking(move || target_path(&output_dir, &path)).await?;
    }
    let already_local = fetcher.local_bytes(&entry.hash).await?;
    if on_event(DownloadEvent::FileStarted {
        path,
        size: entry.size,
        already_local,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    let mut previous = 0;
    let stats = connection
        .fetch_blob(&entry.hash, &mut |received| {
            let new_bytes = received.saturating_sub(previous);
            previous = received;
            if let Some(kind) = receiving.on_received(new_bytes, || connection.usage()) {
                on_event(DownloadEvent::PathChanged { path: kind })?;
            }
            on_event(DownloadEvent::Progress { path, received })
        })
        .await
        .map_err(cancelled)?;

    let name = data_encoding::HEXLOWER.encode(&entry.hash);
    let ciphertext = partial.join(format!("{name}.blob"));
    let plaintext = partial.join(format!("{name}.plain"));
    for leftover in [&ciphertext, &plaintext] {
        if let Err(error) = tokio::fs::remove_file(leftover).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(io_error(leftover)(error));
            }
        }
    }
    fetcher.export_blob(&entry.hash, &ciphertext).await?;
    {
        let cipher = cipher.clone();
        let entry = entry.clone();
        let output_dir = output_dir.to_path_buf();
        run_blocking(move || {
            decrypt_into_place(
                cipher.as_ref(),
                &entry,
                &ciphertext,
                &plaintext,
                &output_dir,
            )
        })
        .await?;
    }

    if on_event(DownloadEvent::FileDone {
        path,
        already_local: stats.already_local,
        received: stats.received,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    Ok(stats.received)
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
