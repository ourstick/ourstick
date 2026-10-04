//! What happens to a file once its blob is here (ADR 0014, 0016): it is
//! placed as room allows, and recorded in groups once in place.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::entry::{finish_placing, start_placing, Placing};
use super::fetch_files::{place_of, FileCounts};
use super::fetch_queue::{FetchQueue, Landed, Watch};
use super::path::ReceivingPath;
use super::placing_queue::PlacingQueue;
use super::relay::RelayLimiter;
use super::{record_downloaded, DownloadError, DownloadEvent, RECORD_GROUP};
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp};
use crate::store::{DownloadedFile, Store};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PeerConnection};

/// Starts placing a file whose blob landed, waiting for room as a received
/// one does.
pub(super) async fn place_landed(
    landed: Landed,
    cipher: &Arc<dyn Cipher>,
    output_dir: &Path,
    placing: &mut PlacingQueue<Placing>,
    done: &mut Done<'_>,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<(), DownloadError> {
    let Landed { queued, received } = landed;
    while !placing.has_room_for(queued.entry.size) {
        done.finish(placing.pop(), on_event).await?;
    }
    let started = start_placing(
        cipher,
        &queued.entry,
        &queued.stick_path,
        received,
        output_dir,
        queued.replaceable,
    );
    placing.push(started, queued.entry.hash, queued.entry.size);
    Ok(())
}

/// Waits for every small file in flight to land, and places each.
#[allow(clippy::too_many_arguments)]
pub(super) async fn land_all(
    fetching: &mut FetchQueue<'_, impl Fetcher, impl PeerConnection>,
    receiving: &mut ReceivingPath<'_>,
    limiter: &mut Option<RelayLimiter>,
    received_total: &mut u64,
    cipher: &Arc<dyn Cipher>,
    output_dir: &Path,
    placing: &mut PlacingQueue<Placing>,
    done: &mut Done<'_>,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<(), DownloadError> {
    loop {
        let mut watch = Watch {
            receiving: &mut *receiving,
            limiter: &mut *limiter,
            received_total: &mut *received_total,
        };
        let Some(landed) = fetching.next(&mut watch, on_event).await? else {
            return Ok(());
        };
        place_landed(landed, cipher, output_dir, placing, done, on_event).await?;
    }
}

/// What a placed file leaves behind: its record, and its place for twins.
pub(super) struct Done<'a> {
    pub(super) store: &'a Arc<dyn Store>,
    pub(super) ticket: &'a Ticket,
    pub(super) output_dir: &'a Path,
    pub(super) written_here: &'a mut HashMap<[u8; 32], (PathBuf, FileStamp)>,
    pub(super) counts: &'a mut FileCounts,
    /// Files placed and not recorded yet: recorded in groups (ADR 0014).
    pub(super) unrecorded: Vec<DownloadedFile>,
}

impl Done<'_> {
    /// Waits for `placing`, if any, and records the file.
    pub(super) async fn finish(
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
    pub(super) async fn finish_all(
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
    pub(super) async fn record_now(&mut self) -> Result<(), DownloadError> {
        let files = std::mem::take(&mut self.unrecorded);
        record_downloaded(self.store, self.ticket, files).await
    }

    /// Records the stick's `stick_path`, saved as `entry` (its place here),
    /// as written with `stamp`, or as left changed (`None`): with the next
    /// group, once [`RECORD_GROUP`] are waiting.
    pub(super) async fn record(
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
