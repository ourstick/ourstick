//! The second half of a download: every file of the plan, batch by batch
//! (`batches`). A batch's store is used while its files are fetched and
//! deleted once they are all in place. Files are placed (decrypted into
//! the folder) while the next ones are received: small ones up to 16 at
//! once, a big one alone (`placing_queue`, ADR 0014). They are recorded in
//! groups, each only once it is in place.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::batches::{plan_batches, BATCH_BYTES, BATCH_FILES};
use super::entry::{copy_file, finish_placing, start_placing, Placing};
use super::fetch_one::receive_file;
use super::folder::FilePlan;
use super::path::ReceivingPath;
use super::placing_queue::PlacingQueue;
use super::relay::RelayLimiter;
use super::{record_downloaded, DownloadError, DownloadEvent, RECORD_GROUP};
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
    pub(super) left_out: usize,
    pub(super) not_chosen: usize,
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
    saved: &[String],
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
    let mut placing: PlacingQueue<Placing> = PlacingQueue::default();
    let mut done = Done {
        store,
        ticket,
        output_dir,
        written_here: &mut written_here,
        counts: &mut counts,
        unrecorded: Vec::new(),
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
                // The entry as it is saved here: its place in the folder for
                // everything on disk (and the events), its path in the stick
                // for its record.
                let stick_path = manifest.entries[index].path.as_str();
                let placed = Entry {
                    path: saved[index].clone(),
                    ..manifest.entries[index].clone()
                };
                let entry = &placed;
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
                    // Told once per top-level name, before the files (ADR 0012).
                    FilePlan::NotChosen => {
                        done.counts.not_chosen += 1;
                        continue;
                    }
                    FilePlan::LeftOut => {
                        done.counts.left_out += 1;
                        DownloadEvent::FileLeftOut {
                            path: &manifest.entries[index].path,
                            size: entry.size,
                        }
                    }
                    FilePlan::Download { replaceable } => {
                        // The same blob as a file being placed: wait for the
                        // last of them, then copy it (and never share its file
                        // in `partial`, which a fetch would clear too).
                        while placing.holds_blob(&entry.hash) {
                            done.finish(placing.pop(), on_event).await?;
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
                            Some((written, told)) => {
                                // In place: recorded even if the user stopped
                                // at its telling, as a placed file is.
                                done.record(stick_path, entry, written).await?;
                                if told.is_break() {
                                    return Err(DownloadError::Cancelled);
                                }
                            }
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
                                // Small files several at once, a big one alone
                                // (ADR 0014).
                                while !placing.has_room_for(entry.size) {
                                    done.finish(placing.pop(), on_event).await?;
                                }
                                let started = start_placing(
                                    cipher,
                                    entry,
                                    stick_path,
                                    received,
                                    output_dir,
                                    replaceable,
                                );
                                placing.push(started, entry.hash, entry.size);
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
            // it) and recorded: its blobs are no longer needed.
            done.finish_all(&mut placing, on_event).await?;
            done.record_now().await?;
            if fetches {
                fetcher.drop_batch(&batch.name).await?;
            }
        }
        Ok(())
    }
    .await;
    // On an error too, every file being placed is finished and recorded, so
    // none is left running and the next run finds them in place.
    let finished = done.finish_all(&mut placing, on_event).await;
    // What is placed is recorded, whatever ended the round.
    let recorded = done.record_now().await;
    result?;
    finished?;
    recorded?;
    Ok(counts)
}

/// What a placed file leaves behind: its record, and its place for twins.
struct Done<'a> {
    store: &'a Arc<dyn Store>,
    ticket: &'a Ticket,
    output_dir: &'a Path,
    written_here: &'a mut HashMap<[u8; 32], (PathBuf, FileStamp)>,
    counts: &'a mut FileCounts,
    /// Files placed and not recorded yet: recorded in groups (ADR 0014).
    unrecorded: Vec<DownloadedFile>,
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
        let stick_path = placing.stick_path().to_owned();
        let (written, told) = finish_placing(placing, on_event).await?;
        // In place: recorded even if the user stopped at its telling.
        self.record(&stick_path, &entry, written).await?;
        if told.is_break() {
            return Err(DownloadError::Cancelled);
        }
        Ok(())
    }

    /// Waits for every file being placed, oldest first, finishing each even
    /// after one failed; the first failure is returned.
    async fn finish_all(
        &mut self,
        placing: &mut PlacingQueue<Placing>,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<(), DownloadError> {
        let mut first = Ok(());
        while !placing.is_empty() {
            let finished = self.finish(placing.pop(), on_event).await;
            if first.is_ok() {
                first = finished;
            }
        }
        first
    }

    /// Records the files placed so far.
    async fn record_now(&mut self) -> Result<(), DownloadError> {
        let files = std::mem::take(&mut self.unrecorded);
        record_downloaded(self.store, self.ticket, files).await
    }

    /// Records the stick's `stick_path`, saved as `entry` (its place here),
    /// as written with `stamp`, or as left changed (`None`): with the next
    /// group, once [`RECORD_GROUP`] are waiting.
    async fn record(
        &mut self,
        stick_path: &str,
        entry: &Entry,
        stamp: Option<FileStamp>,
    ) -> Result<(), DownloadError> {
        match stamp {
            Some(stamp) => {
                self.written_here
                    .insert(entry.hash, (place_of(self.output_dir, &entry.path), stamp));
                let file = DownloadedFile {
                    path: stick_path.to_owned(),
                    saved_as: entry.path.clone(),
                    stamp,
                    blob_hash: entry.hash,
                };
                self.unrecorded.push(file);
                if self.unrecorded.len() >= RECORD_GROUP {
                    self.record_now().await?;
                }
                Ok(())
            }
            None => {
                self.counts.changed_here += 1;
                Ok(())
            }
        }
    }
}
