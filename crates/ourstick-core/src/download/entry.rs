//! The second half of downloading one file: placing it (decrypt it into
//! place, on the blocking pool), once `fetch_one` received it. The caller
//! receives the next files while this one is placed. A file whose blob this
//! download already turned into another file is copied from that file
//! instead.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

use super::fetch_one::Received;
use super::files::{copy_into_place, decrypt_into_place};
use super::{io_error, DownloadError, DownloadEvent};
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp};

/// A file being decrypted into place on the blocking pool.
pub(super) struct Placing {
    /// The entry as it is saved here (its place in the folder).
    entry: Entry,
    /// Its path in the stick, for its record.
    stick_path: String,
    already_local: u64,
    received: u64,
    task: tokio::task::JoinHandle<Result<Option<FileStamp>, DownloadError>>,
}

impl Placing {
    pub(super) fn entry(&self) -> &Entry {
        &self.entry
    }

    pub(super) fn stick_path(&self) -> &str {
        &self.stick_path
    }
}

/// Starts decrypting `received` into place; see [`finish_placing`].
pub(super) fn start_placing(
    cipher: &Arc<dyn Cipher>,
    entry: &Entry,
    stick_path: &str,
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
        stick_path: stick_path.to_owned(),
        already_local: received.already_local,
        received: received.received,
        task,
    }
}

/// Waits for `placing` and says how it went: the file's stamp once in place,
/// `None` if the file changed in the folder while it downloaded (left as it
/// is).
///
/// The file is in place before it is told: whether the user stopped at the
/// telling comes back beside it, so the caller records it either way.
pub(super) async fn finish_placing(
    placing: Placing,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<(Option<FileStamp>, ControlFlow<()>), DownloadError> {
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
    let told = on_event(event);
    Ok((written, told))
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
) -> Result<Option<(Option<FileStamp>, ControlFlow<()>)>, DownloadError> {
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
    // Told after it is in place: whether the user stopped there comes back
    // beside it, for the caller to record it first.
    let told = on_event(event);
    Ok(copied.map(|written| (written, told)))
}
