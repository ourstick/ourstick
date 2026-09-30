//! The second half of a download: every file of the plan, batch by batch
//! (`batches`). A batch's store is used while its files are fetched and
//! deleted once they are all in place. Each file is placed (decrypted into
//! the folder) while the next one is received; at most one waits to be
//! placed.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::batches::{plan_batches, BATCH_BYTES, BATCH_FILES};
use super::entry::{copy_file, finish_placing, receive_file, start_placing, Placing};
use super::folder::FilePlan;
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{record_downloaded, DownloadError, DownloadEvent};
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp, Manifest};
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
    receiving: &mut ReceivingPath<'_>,
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
    let mut placing: Option<Placing> = None;
    let mut done = Done {
        store,
        ticket,
        output_dir,
        written_here: &mut written_here,
        counts: &mut counts,
    };
    let result = async {
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
                        done.counts.already_here += 1;
                        DownloadEvent::FileAlreadyHere {
                            path: &entry.path,
                            size: entry.size,
                        }
                    }
                    FilePlan::ChangedHere => {
                        done.counts.changed_here += 1;
                        DownloadEvent::FileChangedHere {
                            path: &entry.path,
                            size: entry.size,
                        }
                    }
                    FilePlan::Download { replaceable } => {
                        // The same blob as the file being placed: wait for it,
                        // then copy it (and never share its file in `partial`).
                        if placing.as_ref().is_some_and(|p| *p.hash() == entry.hash) {
                            done.finish(placing.take(), on_event).await?;
                        }
                        let mut copied = None;
                        if let Some((source, stamp)) = done.written_here.get(&entry.hash).cloned() {
                            copied = copy_file(
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
                        match copied {
                            Some(written) => done.record(entry, written).await?,
                            None => {
                                let received = receive_file(
                                    fetcher,
                                    connection,
                                    entry,
                                    output_dir,
                                    partial,
                                    receiving,
                                    limiter,
                                    received_total,
                                    on_event,
                                )
                                .await?;
                                // One file placed at a time.
                                done.finish(placing.take(), on_event).await?;
                                placing = Some(start_placing(
                                    cipher,
                                    entry,
                                    received,
                                    output_dir,
                                    replaceable,
                                ));
                            }
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
            done.finish(placing.take(), on_event).await?;
            if fetches {
                fetcher.drop_batch(&batch.name).await?;
            }
        }
        Ok(())
    }
    .await;
    // On an error too, a file being placed is finished and recorded, so the
    // next run finds it in place.
    let finished = done.finish(placing.take(), on_event).await;
    result?;
    finished?;
    Ok(counts)
}

/// What a placed file leaves behind: its record, and its place for twins.
struct Done<'a> {
    store: &'a Arc<dyn Store>,
    ticket: &'a Ticket,
    output_dir: &'a Path,
    written_here: &'a mut HashMap<[u8; 32], (PathBuf, FileStamp)>,
    counts: &'a mut FileCounts,
}

impl Done<'_> {
    /// Waits for `placing`, if any, and records the file.
    async fn finish(
        &mut self,
        placing: Option<Placing>,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<(), DownloadError> {
        let Some(placing) = placing else {
            return Ok(());
        };
        let entry = placing.entry().clone();
        let written = finish_placing(placing, on_event).await?;
        self.record(&entry, written).await
    }

    /// Records `entry` as written with `stamp`, or as left changed (`None`).
    async fn record(
        &mut self,
        entry: &Entry,
        stamp: Option<FileStamp>,
    ) -> Result<(), DownloadError> {
        match stamp {
            Some(stamp) => {
                self.written_here
                    .insert(entry.hash, (place_of(self.output_dir, &entry.path), stamp));
                let file = DownloadedFile {
                    path: entry.path.clone(),
                    stamp,
                    blob_hash: entry.hash,
                };
                record_downloaded(self.store, self.ticket, file).await
            }
            None => {
                self.counts.changed_here += 1;
                Ok(())
            }
        }
    }
}
