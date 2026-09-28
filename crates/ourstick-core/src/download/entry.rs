//! Downloading one file of the manifest: check its path, fetch its blob
//! (watching which path the data arrives on), decrypt it into place.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

use super::files::{decrypt_into_place, target_path};
use super::path::ReceivingPath;
use super::{cancelled, io_error, DownloadError, DownloadEvent};
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::manifest::Entry;
use crate::transport::{Fetcher, PathKind, PeerConnection};

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_file(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    cipher: &Arc<dyn Cipher>,
    entry: &Entry,
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath,
    direct_only: bool,
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
    let mut went_through_relay = false;
    let stats = connection
        .fetch_blob(&entry.hash, &mut |received| {
            let new_bytes = received.saturating_sub(previous);
            previous = received;
            if let Some(check) = receiving.on_received(new_bytes, || connection.usage()) {
                if check.changed {
                    on_event(DownloadEvent::PathChanged { path: check.kind })?;
                }
                // Only a direct path is acceptable: stop, reconnect, try again.
                if direct_only && check.kind == PathKind::Relay {
                    went_through_relay = true;
                    return ControlFlow::Break(());
                }
            }
            on_event(DownloadEvent::Progress { path, received })
        })
        .await
        .map_err(|error| {
            if went_through_relay {
                DownloadError::DirectLost
            } else {
                cancelled(error)
            }
        })?;

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
