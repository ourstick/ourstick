//! A download's room in the space ledger (ADR 0008): claimed once the
//! download knows what it will write, smaller as files land, given back when
//! the download returns. A download that does not fit does not start. The
//! one place every program (the command line, the app) keeps a download's
//! room.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{claim_download, Disks, SpaceError, SystemDisks};
use crate::download::{DownloadEvent, SCRATCH_BYTES};
use crate::stick_id::StickId;
use crate::store::{ClaimKind, Store};

/// The room one download of a stick holds, while this program holds the
/// stick's download lock. Its calls do short blocking work (a disk's
/// figures, one transaction): from a multi-threaded tokio runtime.
pub struct DownloadRoom {
    disks: Box<dyn Disks>,
    store: Arc<dyn Store>,
    data_dir: PathBuf,
    stick_id: StickId,
    output: PathBuf,
    /// Bytes still to write into the folder, and what the claim says now
    /// (written to the store in steps, not at every file).
    to_come: Mutex<(u64, u64)>,
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
        Self::with_disks(Box::new(SystemDisks), store, data_dir, stick_id, output)
    }

    pub(crate) fn with_disks(
        disks: Box<dyn Disks>,
        store: Arc<dyn Store>,
        data_dir: PathBuf,
        stick_id: StickId,
        output: PathBuf,
    ) -> Self {
        Self {
            disks,
            store,
            data_dir,
            stick_id,
            output,
            to_come: Mutex::new((0, 0)),
            refused: Mutex::new(None),
        }
    }

    /// Follows a download's events: claims at `Planned` (breaking if it does
    /// not fit), shrinks as a file lands, fetched or copied from another of
    /// its files, and gives everything back while it waits for the owner's
    /// next version (that can take hours). Pass every event.
    pub fn on_event(&self, event: &DownloadEvent<'_>) -> ControlFlow<()> {
        match event {
            DownloadEvent::Planned { bytes } => return self.planned(*bytes),
            DownloadEvent::FileReceived { size, .. }
            | DownloadEvent::FileCopiedHere { size, .. } => self.landed(*size),
            DownloadEvent::WaitingForMore { .. } => self.release(),
            _ => {}
        }
        ControlFlow::Continue(())
    }

    /// The download will write `needed` bytes: claims them, and room for
    /// its batches on the data folder's disk, or stops it if they do not fit.
    /// Nothing to write holds nothing (an up-to-date stick is never refused
    /// for room it does not use).
    pub fn planned(&self, needed: u64) -> ControlFlow<()> {
        *lock(&self.refused) = None;
        if needed == 0 {
            self.release();
            return ControlFlow::Continue(());
        }
        let claimed = tokio::task::block_in_place(|| {
            claim_download(
                self.disks.as_ref(),
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
                *lock(&self.to_come) = (needed, needed);
                ControlFlow::Continue(())
            }
            Err(error) => {
                *lock(&self.refused) = Some(error);
                ControlFlow::Break(())
            }
        }
    }

    /// `size` bytes landed in the folder: the claim holds that much less,
    /// written to the store once it is [`UPDATE_STEP`] less (or nothing is
    /// left), not at every small file. A fetched file counts at
    /// `FileReceived`, a moment before it is placed: the claim may be short
    /// by about one file per download while it is.
    pub fn landed(&self, size: u64) {
        let left = {
            let mut to_come = lock(&self.to_come);
            let (left, claimed) = &mut *to_come;
            *left = left.saturating_sub(size);
            if *claimed - *left < UPDATE_STEP && *left > 0 {
                return;
            }
            *claimed = *left;
            *left
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
        *lock(&self.to_come) = (0, 0);
        for kind in [ClaimKind::Download, ClaimKind::Scratch] {
            let removed =
                tokio::task::block_in_place(|| self.store.remove_space_claim(&self.stick_id, kind));
            if let Err(error) = removed {
                tracing::warn!(%error, "the download's room could not be given back");
            }
        }
    }
}

/// How much smaller a claim gets before the store hears of it: one write per
/// 64 MiB landed, not one per file (a stick of many small files).
pub const UPDATE_STEP: u64 = 64 << 20;

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
    use std::path::Path;

    const GB: u64 = 1 << 30;

    /// Every path on one disk with `free` of 1000 GB.
    struct OneDisk(u64);

    impl Disks for OneDisk {
        fn key(&self, _path: &Path) -> std::io::Result<String> {
            Ok("d".to_owned())
        }
        fn free(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(self.0)
        }
        fn total(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(1000 * GB)
        }
    }

    struct World {
        data: tempfile::TempDir,
        store: Arc<dyn Store>,
        room: DownloadRoom,
        _lock: crate::locks::Lock,
    }

    /// A room on a disk with `free` bytes, its stick's download lock held.
    fn world(free: u64) -> World {
        let data = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        let lock = download_lock(data.path(), &stick_id).unwrap();
        let room = DownloadRoom::with_disks(
            Box::new(OneDisk(free)),
            store.clone(),
            data.path().to_path_buf(),
            stick_id,
            data.path().join("out"),
        );
        World {
            data,
            store,
            room,
            _lock: lock,
        }
    }

    fn held(world: &World, kind: ClaimKind) -> Option<u64> {
        world
            .store
            .space_claims()
            .unwrap()
            .into_iter()
            .find(|claim| claim.kind == kind)
            .map(|claim| claim.bytes)
    }

    fn received(size: u64) -> DownloadEvent<'static> {
        DownloadEvent::FileReceived {
            path: "a",
            size,
            already_local: 0,
            received: size,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_claims_its_room_and_its_scratch_and_gives_both_back() {
        let world = world(500 * GB);
        let room = &world.room;
        assert!(room
            .on_event(&DownloadEvent::Planned { bytes: 10 * GB })
            .is_continue());
        assert_eq!(held(&world, ClaimKind::Download), Some(10 * GB));
        assert_eq!(held(&world, ClaimKind::Scratch), Some(SCRATCH_BYTES));
        // Small files are written to the store in steps.
        assert!(room.on_event(&received(UPDATE_STEP / 2)).is_continue());
        assert_eq!(held(&world, ClaimKind::Download), Some(10 * GB));
        let copied = DownloadEvent::FileCopiedHere {
            path: "b",
            size: 4 * GB,
        };
        assert!(room.on_event(&copied).is_continue());
        assert_eq!(
            held(&world, ClaimKind::Download),
            Some(6 * GB - UPDATE_STEP / 2)
        );
        // The last file always tells the store.
        room.landed(10 * GB);
        assert_eq!(held(&world, ClaimKind::Download), Some(0));
        room.release();
        room.release();
        assert!(world.store.space_claims().unwrap().is_empty());
        assert!(world.data.path().exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_plan_replaces_the_last_one() {
        let world = world(500 * GB);
        assert!(world.room.planned(10 * GB).is_continue());
        assert!(world.room.planned(3 * GB).is_continue());
        assert_eq!(held(&world, ClaimKind::Download), Some(3 * GB));
        assert_eq!(world.store.space_claims().unwrap().len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_that_does_not_fit_says_how_much_and_only_until_one_fits() {
        let world = world(120 * GB);
        // 120 GB free, 20 GB of it the system's: 100 GB left.
        assert!(world.room.planned(200 * GB).is_break());
        assert!(world.store.space_claims().unwrap().is_empty());
        assert!(world.room.planned(200 * GB).is_break());
        assert!(world.room.planned(GB).is_continue());
        // A refusal is not told after a round that fitted.
        assert!(world.room.refusal().is_none());
        assert!(world.room.planned(200 * GB).is_break());
        assert!(matches!(
            world.room.refusal(),
            Some(SpaceError::DoesNotFit { needed, .. }) if needed > 200 * GB
        ));
        assert!(world.room.refusal().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_to_write_holds_nothing_even_on_a_full_disk() {
        let world = world(5 * GB);
        assert!(world.room.planned(0).is_continue());
        assert!(world.room.refusal().is_none());
        assert!(world.store.space_claims().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn waiting_for_the_owner_holds_nothing() {
        let world = world(500 * GB);
        assert!(world.room.planned(10 * GB).is_continue());
        let waiting = DownloadEvent::WaitingForMore { seq: 3 };
        assert!(world.room.on_event(&waiting).is_continue());
        assert!(world.store.space_claims().unwrap().is_empty());
    }
}
