//! The second half of a download: every file of the plan, batch by batch
//! (`batches`). A batch's store is used while its files are fetched and
//! deleted once they are all in place.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::batches::{plan_batches, BATCH_BYTES, BATCH_FILES};
use super::entry::{copy_file, download_file};
use super::folder::FilePlan;
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{record_downloaded, DownloadError, DownloadEvent};
use crate::cipher::Cipher;
use crate::manifest::{FileStamp, Manifest};
use crate::store::{DownloadedFile, Store};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PeerConnection};

/// What the files of one download came to.
#[derive(Debug, Default)]
pub(super) struct FileCounts {
    pub(super) already_here: usize,
    pub(super) changed_here: usize,
}

/// Where an entry lands under `output_dir` (its path was checked before).
fn place_of(output_dir: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(output_dir.to_path_buf(), |place, part| place.join(part))
}

/// Fetches and writes every entry `files` says to, batch by batch.
#[allow(clippy::too_many_arguments)]
pub(super) async fn fetch_files(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    cipher: &Arc<dyn Cipher>,
    manifest: &Manifest,
    files: &[FilePlan],
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath,
    limiter: &mut Option<RelayLimiter>,
    received_total: &mut u64,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<FileCounts, DownloadError> {
    let batches = plan_batches(&manifest.entries, BATCH_BYTES, BATCH_FILES);
    let names: Vec<String> = batches.iter().map(|batch| batch.name.clone()).collect();
    // Batches of an older version, or of an interrupted run of one, go.
    fetcher.keep_only_batches(&names).await?;

    let mut counts = FileCounts::default();
    // Files this download wrote, by blob: a later entry with the same blob is
    // copied from them.
    let mut written_here: HashMap<[u8; 32], (PathBuf, FileStamp)> = HashMap::new();
    for batch in &batches {
        let fetches = files[batch.entries.clone()]
            .iter()
            .any(|file| matches!(file, FilePlan::Download { .. }));
        if fetches {
            fetcher.use_batch(&batch.name).await?;
        }
        for index in batch.entries.clone() {
            let entry = &manifest.entries[index];
            let event = match files[index] {
                FilePlan::AlreadyHere => {
                    counts.already_here += 1;
                    DownloadEvent::FileAlreadyHere {
                        path: &entry.path,
                        size: entry.size,
                    }
                }
                FilePlan::ChangedHere => {
                    counts.changed_here += 1;
                    DownloadEvent::FileChangedHere {
                        path: &entry.path,
                        size: entry.size,
                    }
                }
                FilePlan::Download { replaceable } => {
                    let mut written = None;
                    if let Some((source, stamp)) = written_here.get(&entry.hash).cloned() {
                        written = copy_file(
                            entry,
                            &source,
                            stamp,
                            output_dir,
                            partial,
                            replaceable,
                            on_event,
                        )
                        .await?;
                    }
                    let written = match written {
                        Some(written) => written,
                        None => {
                            download_file(
                                fetcher,
                                connection,
                                cipher,
                                entry,
                                output_dir,
                                partial,
                                receiving,
                                limiter,
                                replaceable,
                                received_total,
                                on_event,
                            )
                            .await?
                        }
                    };
                    match written {
                        Some(stamp) => {
                            written_here
                                .insert(entry.hash, (place_of(output_dir, &entry.path), stamp));
                            let file = DownloadedFile {
                                path: entry.path.clone(),
                                stamp,
                                blob_hash: entry.hash,
                            };
                            record_downloaded(store, ticket, file).await?;
                        }
                        None => counts.changed_here += 1,
                    }
                    continue;
                }
            };
            if on_event(event).is_break() {
                return Err(DownloadError::Cancelled);
            }
        }
        // Every file of the batch is in place (or left as the user changed
        // it): its blobs are no longer needed.
        if fetches {
            fetcher.drop_batch(&batch.name).await?;
        }
    }
    Ok(counts)
}
