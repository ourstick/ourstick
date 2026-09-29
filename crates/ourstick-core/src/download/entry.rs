//! Downloading one file of the manifest: check its path, fetch its blob
//! (watching which path the data arrives on), decrypt it into place.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use super::files::{decrypt_into_place, target_path};
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{cancelled, io_error, DownloadError, DownloadEvent};
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp};
use crate::transport::{Fetcher, PathKind, PeerConnection, GO};

/// How often, in received bytes, the relay limit looks at the relay count.
const LIMIT_SAMPLE: u64 = 256 * 1024;

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_file(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    cipher: &Arc<dyn Cipher>,
    entry: &Entry,
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath,
    limiter: &mut Option<RelayLimiter>,
    replaceable: Option<FileStamp>,
    received_total: &mut u64,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Option<FileStamp>, DownloadError> {
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
    let mut unsampled = 0;
    let stats = connection
        .fetch_blob(&entry.hash, &mut |received| {
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
    let written = {
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
                replaceable,
            )
        })
        .await?
    };

    let event = match written {
        Some(_) => DownloadEvent::FileDone {
            path,
            already_local: stats.already_local,
            received: stats.received,
        },
        // Changed in the folder while it downloaded.
        None => DownloadEvent::FileChangedHere {
            path,
            size: entry.size,
        },
    };
    if on_event(event).is_break() {
        return Err(DownloadError::Cancelled);
    }
    Ok(written)
}
