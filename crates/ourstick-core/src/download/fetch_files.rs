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
use super::entry::{copy_file, start_placing, Placing};
use super::fetch_one::{check_place, receive_file};
use super::fetch_queue::{FetchQueue, Queued, Watch};
use super::folder::FilePlan;
use super::landing::{land_all, place_landed, Done};
use super::path::ReceivingPath;
use super::placing_queue::{PlacingQueue, QUEUED_FILE_BYTES};
use super::relay::RelayLimiter;
use super::{DownloadError, DownloadEvent};
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp, Manifest};
use crate::store::Store;
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
pub(super) fn place_of(output_dir: &Path, path: &str) -> PathBuf {
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
    saved: &super::folder::SavedAt,
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
    // Small files' blobs, several at once (ADR 0016).
    let mut fetching = FetchQueue::new(fetcher, connection, partial);
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
                    path: saved.of(index, stick_path).to_owned(),
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
                        // The same blob as a file being fetched: wait for it
                        // to land (it is then placed, and copied below).
                        while fetching.holds(&entry.hash) {
                            let mut watch = Watch {
                                receiving: &mut *receiving,
                                limiter: &mut *limiter,
                                received_total: &mut *received_total,
                            };
                            if let Some(landed) = fetching.next(&mut watch, on_event).await? {
                                place_landed(
                                    landed,
                                    cipher,
                                    output_dir,
                                    &mut placing,
                                    &mut done,
                                    on_event,
                                )
                                .await?;
                            }
                        }
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
                            None if entry.size <= QUEUED_FILE_BYTES => {
                                // Small: fetched with others (ADR 0016).
                                check_place(output_dir, &entry.path).await?;
                                while !fetching.has_room() {
                                    let mut watch = Watch {
                                        receiving: &mut *receiving,
                                        limiter: &mut *limiter,
                                        received_total: &mut *received_total,
                                    };
                                    if let Some(landed) =
                                        fetching.next(&mut watch, on_event).await?
                                    {
                                        place_landed(
                                            landed,
                                            cipher,
                                            output_dir,
                                            &mut placing,
                                            &mut done,
                                            on_event,
                                        )
                                        .await?;
                                    }
                                }
                                fetching.push(Queued::new(
                                    entry.clone(),
                                    stick_path.to_owned(),
                                    replaceable,
                                ));
                            }
                            None => {
                                // Big: alone, once the small ones landed.
                                land_all(
                                    &mut fetching,
                                    receiving,
                                    limiter,
                                    received_total,
                                    cipher,
                                    output_dir,
                                    &mut placing,
                                    &mut done,
                                    on_event,
                                )
                                .await?;
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
            // it) and recorded: its blobs are no longer needed. Every fetch of
            // it has ended before its store goes.
            land_all(
                &mut fetching,
                receiving,
                limiter,
                received_total,
                cipher,
                output_dir,
                &mut placing,
                &mut done,
                on_event,
            )
            .await?;
            done.finish_all(&mut placing, on_event).await?;
            done.record_now().await?;
            if fetches {
                fetcher.drop_batch(&batch.name).await?;
            }
        }
        Ok(())
    }
    .await;
    // On an error, every fetch still going is stopped and waited for: none
    // outlives the round (what arrived stays in the batch's store).
    if result.is_err() && !fetching.is_empty() {
        let _ = fetching.stop(DownloadError::Cancelled).await;
    }
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
