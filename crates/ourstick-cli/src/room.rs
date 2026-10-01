//! A download's room in the space ledger (ADR 0008): claimed once the
//! download knows what it will write, smaller as files land, gone with the
//! download's lock. A download that does not fit does not start.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ourstick_core::download::SCRATCH_BYTES;
use ourstick_core::space::{claim_download, SpaceError, SystemDisks};
use ourstick_core::{ClaimKind, StickId, Store};

use crate::display::bytes;

/// The room one `get` holds.
pub struct Room {
    store: Arc<dyn Store>,
    data_dir: PathBuf,
    stick_id: StickId,
    output: PathBuf,
    /// Bytes still to write into the folder.
    to_come: Mutex<u64>,
    /// Why the last version did not start, if it did not fit.
    refused: Mutex<Option<String>>,
}

impl Room {
    /// The room of downloading `stick_id` into `output`; this program holds
    /// the stick's download lock.
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

    /// The download will write `needed` bytes: claims them (and room for its
    /// batches on the data folder's disk), or stops it if they do not fit.
    pub fn planned(&self, needed: u64) -> ControlFlow<()> {
        // Short, blocking work (a disk's figures, one transaction).
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
            Err(SpaceError::DoesNotFit { needed, available }) => {
                *lock(&self.refused) = Some(format!(
                    "it does not fit: it needs {}, and {} are left on that disk \
                     (OurStick leaves some room for the system and counts what other \
                     shares and downloads hold). Free some room, or choose another \
                     folder with --out.",
                    bytes(needed),
                    bytes(available)
                ));
                ControlFlow::Break(())
            }
            Err(error) => {
                *lock(&self.refused) = Some(error.to_string());
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

    /// Why the download did not start, if it did not fit.
    pub fn refusal(&self) -> Option<String> {
        lock(&self.refused).take()
    }

    /// Gives the room back (it would go with the lock anyway).
    pub fn release(&self) {
        let removed = self
            .store
            .remove_space_claim(&self.stick_id, ClaimKind::Download);
        if let Err(error) = removed {
            tracing::warn!(%error, "the download's room could not be given back");
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
    use ourstick_core::locks::download_lock;
    use ourstick_core::MemoryStore;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_claims_its_room_and_gives_it_back() {
        let data = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let stick_id = ourstick_core::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let _lock = download_lock(data.path(), &stick_id).unwrap();
        let room = Room::new(
            store.clone(),
            data.path().to_path_buf(),
            stick_id,
            output.path().to_path_buf(),
        );
        assert!(room.planned(1_000).is_continue());
        room.landed(400);
        assert_eq!(store.space_claims().unwrap()[0].bytes, 600);
        room.release();
        assert!(store.space_claims().unwrap().is_empty());
        // Far more than any disk: refused, and says why.
        assert!(room.planned(u64::MAX / 4).is_break());
        assert!(room.refusal().unwrap().starts_with("it does not fit"));
        assert!(store.space_claims().unwrap().is_empty());
    }
}
