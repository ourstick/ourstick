//! Downloading one file of the manifest in two halves: receiving it (check
//! its path, fetch its blob while watching which path the data arrives on
//! and waiting while the owner prepares it, take the blob out of the store),
//! then placing it (decrypt it into place, on the blocking pool). The caller
//! receives the next file while this one is placed. A file whose blob this
//! download already turned into another file is copied from that file
//! instead.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use super::files::{copy_into_place, decrypt_into_place, target_path};
use super::not_ready::Waiting;
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{cancelled, io_error, DownloadError, DownloadEvent};
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp};
use crate::transport::{Fetcher, PathKind, PeerConnection, TransportError, GO};

/// How often, in received bytes, the relay limit looks at the relay count.
const LIMIT_SAMPLE: u64 = 256 * 1024;

/// A file whose blob is here, out of the store, not placed yet.
#[derive(Debug)]
pub(super) struct Received {
    ciphertext: PathBuf,
    plaintext: PathBuf,
    already_local: u64,
    received: u64,
}

/// Fetches the blob of `entry` and takes it out of the store into `partial`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn receive_file(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    entry: &Entry,
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath,
    limiter: &mut Option<RelayLimiter>,
    received_total: &mut u64,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Received, DownloadError> {
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

    let mut waiting: Option<Waiting> = None;
    let stats = loop {
        let mut previous = 0;
        let mut unsampled = 0;
        let mut on_progress = |received: u64| {
            let new_bytes = received.saturating_sub(previous);
            previous = received;
            *received_total += new_bytes;
            if let Some(check) = receiving.on_received(new_bytes, || connection.usage()) {
                if check.changed {
                    on_event(DownloadEvent::PathChanged { path: check.kind })?;
                }
                if let Some(limiter) = limiter.as_mut() {
                    if limiter.announce(check.kind == PathKind::Relay) {
                        on_event(DownloadEvent::RelayLimited {
                            bits_per_second: limiter.bits_per_second(),
                        })?;
                    }
                }
            }
            on_event(DownloadEvent::Progress { path, received })?;
            // ADR 0006: data through a relay is slowed to the limit, never
            // stopped. Sampled now and then: `usage` is not free.
            unsampled += new_bytes;
            match limiter.as_mut() {
                Some(limiter) if unsampled >= LIMIT_SAMPLE => {
                    unsampled = 0;
                    ControlFlow::Continue(limiter.wait(connection.usage().relay, Instant::now()))
                }
                _ => GO,
            }
        };
        match connection.fetch_blob(&entry.hash, &mut on_progress).await {
            Ok(stats) => break stats,
            Err(TransportError::NotReady) => {
                let now = Instant::now();
                if waiting.is_none() && on_event(DownloadEvent::SenderPreparing { path }).is_break()
                {
                    return Err(DownloadError::Cancelled);
                }
                let pause = waiting
                    .get_or_insert_with(|| Waiting::new(now))
                    .next_pause(now)
                    .ok_or(DownloadError::Transport(TransportError::NotReady))?;
                tokio::time::sleep(pause).await;
            }
            Err(error) => return Err(cancelled(error)),
        }
    };

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
    if on_event(DownloadEvent::FileReceived {
        path,
        size: entry.size,
        already_local: stats.already_local,
        received: stats.received,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    Ok(Received {
        ciphertext,
        plaintext,
        already_local: stats.already_local,
        received: stats.received,
    })
}

/// A file being decrypted into place on the blocking pool.
pub(super) struct Placing {
    entry: Entry,
    already_local: u64,
    received: u64,
    task: tokio::task::JoinHandle<Result<Option<FileStamp>, DownloadError>>,
}

impl Placing {
    /// The blob this placement reads (its file in `partial` is named after it).
    pub(super) fn hash(&self) -> &[u8; 32] {
        &self.entry.hash
    }

    pub(super) fn entry(&self) -> &Entry {
        &self.entry
    }
}

/// Starts decrypting `received` into place; see [`finish_placing`].
pub(super) fn start_placing(
    cipher: &Arc<dyn Cipher>,
    entry: &Entry,
    received: Received,
    output_dir: &Path,
    replaceable: Option<FileStamp>,
) -> Placing {
    let task = {
        let cipher = cipher.clone();
        let entry = entry.clone();
        let output_dir = output_dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            decrypt_into_place(
                cipher.as_ref(),
                &entry,
                &received.ciphertext,
                &received.plaintext,
                &output_dir,
                replaceable,
            )
        })
    };
    Placing {
        entry: entry.clone(),
        already_local: received.already_local,
        received: received.received,
        task,
    }
}

/// Waits for `placing` and says how it went: the file's stamp once in place,
/// `None` if the file changed in the folder while it downloaded (left as it
/// is).
pub(super) async fn finish_placing(
    placing: Placing,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Option<FileStamp>, DownloadError> {
    let written = match placing.task.await {
        Ok(written) => written?,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => panic!("placing a file was cancelled: {error}"),
    };
    let path = placing.entry.path.as_str();
    let event = match written {
        Some(_) => DownloadEvent::FileDone {
            path,
            already_local: placing.already_local,
            received: placing.received,
        },
        None => DownloadEvent::ReceivedButChangedHere { path },
    };
    if on_event(event).is_break() {
        return Err(DownloadError::Cancelled);
    }
    Ok(written)
}

/// Places `entry` by copying `source`, a file this download already wrote from
/// the same blob (identical files share a blob, ADR 0003). Returns `None` if
/// `source` changed since, so the caller fetches the blob instead; otherwise
/// what [`finish_placing`] returns.
pub(super) async fn copy_file(
    entry: &Entry,
    source: &Path,
    source_stamp: FileStamp,
    output_dir: &Path,
    partial: &Path,
    replaceable: Option<FileStamp>,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Option<Option<FileStamp>>, DownloadError> {
    let path = entry.path.as_str();
    let name = data_encoding::HEXLOWER.encode(&entry.hash);
    let plaintext = partial.join(format!("{name}.copy"));
    if let Err(error) = tokio::fs::remove_file(&plaintext).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(io_error(&plaintext)(error));
        }
    }
    let copied = {
        let entry = entry.clone();
        let source = source.to_path_buf();
        let output_dir = output_dir.to_path_buf();
        run_blocking(move || {
            copy_into_place(
                &entry,
                &source,
                source_stamp,
                &plaintext,
                &output_dir,
                replaceable,
            )
        })
        .await?
    };
    let event = match copied {
        None => return Ok(None),
        Some(Some(_)) => DownloadEvent::FileCopiedHere {
            path,
            size: entry.size,
        },
        Some(None) => DownloadEvent::FileChangedHere {
            path,
            size: entry.size,
        },
    };
    if on_event(event).is_break() {
        return Err(DownloadError::Cancelled);
    }
    Ok(copied)
}
