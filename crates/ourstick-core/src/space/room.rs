//! A download's room in the space ledger (ADR 0008): claimed once the
//! download knows what it will write, smaller as files land, given back when
//! the download returns. A download that does not fit does not start. The
//! one place every program (the command line, the app) keeps a download's
//! room.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{claim_download, SpaceError, SystemDisks};
use crate::download::{DownloadEvent, SCRATCH_BYTES};
use crate::stick_id::StickId;
use crate::store::{ClaimKind, Store};

/// The room one download of a stick holds, while this program holds the
/// stick's download lock. Its calls do short blocking work (a disk's
/// figures, one transaction): from a multi-threaded tokio runtime.
pub struct DownloadRoom {
    store: Arc<dyn Store>,
    data_dir: PathBuf,
    stick_id: StickId,
    output: PathBuf,
    /// Bytes still to write into the folder.
    to_come: Mutex<u64>,
    /// Why the last version did not start.
    refused: Mutex<Option<SpaceError>>,
}

impl DownloadRoom {
    pub fn new(
        store: Arc<dyn Store>,
        data_dir: PathBuf,
        stick_id: StickId,
        output: PathBuf,
    ) -> Self {
        Self {
            store,
            data_dir,
            stick_id,
            output,
            to_come: Mutex::new(0),
            refused: Mutex::new(None),
        }
    }

    /// Follows a download's events: claims at `Planned` (breaking if it does
    /// not fit), shrinks as a file lands, fetched or copied from another of
    /// its files. Pass every event.
    pub fn on_event(&self, event: &DownloadEvent<'_>) -> ControlFlow<()> {
        match event {
            DownloadEvent::Planned { bytes } => return self.planned(*bytes),
            DownloadEvent::FileReceived { size, .. }
            | DownloadEvent::FileCopiedHere { size, .. } => self.landed(*size),
            _ => {}
        }
        ControlFlow::Continue(())
    }

    /// The download will write `needed` bytes: claims them, and room for
    /// its batches on the data folder's disk, or stops it if they do not fit.
    pub fn planned(&self, needed: u64) -> ControlFlow<()> {
        let claimed = tokio::task::block_in_place(|| {
            claim_download(
                &SystemDisks,
                self.store.as_ref(),
                &self.data_dir,
                &self.stick_id,
                &self.output,
                needed,
                SCRATCH_BYTES,
            )
        });
        match claimed {
            Ok(()) => {
                *lock(&self.to_come) = needed;
                ControlFlow::Continue(())
            }
            Err(error) => {
                *lock(&self.refused) = Some(error);
                ControlFlow::Break(())
            }
        }
    }

    /// `size` bytes landed in the folder: the claim holds that much less.
    pub fn landed(&self, size: u64) {
        let left = {
            let mut to_come = lock(&self.to_come);
            *to_come = to_come.saturating_sub(size);
            *to_come
        };
        let updated = tokio::task::block_in_place(|| {
            self.store
                .update_space_claim(&self.stick_id, ClaimKind::Download, left)
        });
        if let Err(error) = updated {
            tracing::warn!(%error, "the download's room could not be updated");
        }
    }

    /// Why the last version did not start (once), e.g.
    /// [`SpaceError::DoesNotFit`] with what it needs and what is left.
    pub fn refusal(&self) -> Option<SpaceError> {
        lock(&self.refused).take()
    }

    /// Gives the room back when a download returns (it would go with the
    /// lock anyway): a stick that is up to date holds none.
    pub fn release(&self) {
        *lock(&self.to_come) = 0;
        for kind in [ClaimKind::Download, ClaimKind::Scratch] {
            if let Err(error) = self.store.remove_space_claim(&self.stick_id, kind) {
                tracing::warn!(%error, "the download's room could not be given back");
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locks::download_lock;
    use crate::store::MemoryStore;

    fn room(
        store: &Arc<dyn Store>,
        data: &std::path::Path,
        output: &std::path::Path,
    ) -> DownloadRoom {
        let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        DownloadRoom::new(
            store.clone(),
            data.to_path_buf(),
            stick_id,
            output.to_path_buf(),
        )
    }

    fn held(store: &Arc<dyn Store>, kind: ClaimKind) -> Option<u64> {
        store
            .space_claims()
            .unwrap()
            .into_iter()
            .find(|claim| claim.kind == kind)
            .map(|claim| claim.bytes)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_claims_its_room_and_its_scratch_and_gives_both_back() {
        let data = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let room = room(&store, data.path(), output.path());
        let _lock = download_lock(data.path(), &room.stick_id).unwrap();
        assert!(room
            .on_event(&DownloadEvent::Planned { bytes: 1_000 })
            .is_continue());
        assert_eq!(held(&store, ClaimKind::Download), Some(1_000));
        assert_eq!(held(&store, ClaimKind::Scratch), Some(SCRATCH_BYTES));
        let received = DownloadEvent::FileReceived {
            path: "a",
            size: 400,
            already_local: 0,
            received: 400,
        };
        assert!(room.on_event(&received).is_continue());
        let copied = DownloadEvent::FileCopiedHere {
            path: "b",
            size: 100,
        };
        assert!(room.on_event(&copied).is_continue());
        assert_eq!(held(&store, ClaimKind::Download), Some(500));
        room.release();
        assert!(store.space_claims().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_that_does_not_fit_holds_nothing_and_says_how_much() {
        let data = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let room = room(&store, data.path(), output.path());
        let _lock = download_lock(data.path(), &room.stick_id).unwrap();
        // Far more than any disk.
        assert!(room.planned(u64::MAX / 4).is_break());
        assert!(matches!(
            room.refusal(),
            Some(SpaceError::DoesNotFit { needed, .. }) if needed >= u64::MAX / 4
        ));
        assert!(room.refusal().is_none());
        assert!(store.space_claims().unwrap().is_empty());
    }
}
