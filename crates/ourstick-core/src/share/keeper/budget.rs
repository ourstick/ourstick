//! The room the owner's suitcases may take (ADR 0007): one budget for the
//! device, split between the sticks shared at once and taken when each
//! starts ([`crate::space::claim_cache`]); suitcases shrink what is free.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::space::{claim_cache, Disks, SpaceError};
use crate::stick_id::StickId;
use crate::store::Store;

/// The budget is at least this…
pub const MIN_BUDGET: u64 = 2 << 30;
/// …and at most this.
pub const MAX_BUDGET: u64 = 40 << 30;

#[derive(Debug)]
struct Room {
    total: u64,
    reserved: u64,
}

/// The budget; cheap to clone, and every clone shares it.
#[derive(Debug, Clone)]
pub struct Budget {
    room: Arc<Mutex<Room>>,
}

impl Budget {
    /// A budget of `total` bytes (tests, and a user's choice later).
    pub fn new(total: u64) -> Self {
        Self {
            room: Arc::new(Mutex::new(Room { total, reserved: 0 })),
        }
    }

    /// The budget of the stick `stick_id`, whose lock this program holds,
    /// with its suitcases in `cache_dir`: its part of the device's budget,
    /// claimed in the space ledger while the lock is held; `together` sticks
    /// start at once (1 for one). Blocking.
    pub fn claim(
        disks: &dyn Disks,
        store: &dyn Store,
        data_dir: &Path,
        stick_id: &StickId,
        cache_dir: &Path,
        together: usize,
    ) -> Result<Self, SpaceError> {
        Ok(Self::new(claim_cache(
            disks, store, data_dir, stick_id, cache_dir, together,
        )?))
    }

    fn room(&self) -> MutexGuard<'_, Room> {
        self.room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn total(&self) -> u64 {
        self.room().total
    }

    /// What is not reserved.
    pub fn available(&self) -> u64 {
        let room = self.room();
        room.total.saturating_sub(room.reserved)
    }

    /// Reserves `bytes` if they fit.
    pub fn reserve(&self, bytes: u64) -> bool {
        let mut room = self.room();
        if room.total.saturating_sub(room.reserved) < bytes {
            return false;
        }
        room.reserved += bytes;
        true
    }

    /// Reserves `bytes` whether or not they fit: a file bigger than the
    /// whole budget, once nothing else is reserved.
    pub fn reserve_beyond(&self, bytes: u64) {
        self.room().reserved += bytes;
    }

    /// Gives back what [`Budget::reserve`] took.
    pub fn release(&self, bytes: u64) {
        let mut room = self.room();
        room.reserved = room.reserved.saturating_sub(bytes);
    }

    /// Whether nothing is reserved.
    pub fn is_empty(&self) -> bool {
        self.room().reserved == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_fit_or_wait() {
        let budget = Budget::new(10);
        let shared = budget.clone();
        assert!(budget.reserve(6));
        assert!(!shared.reserve(5), "clones share the budget");
        assert_eq!(shared.available(), 4);
        budget.release(6);
        assert!(budget.is_empty());
        budget.reserve_beyond(25);
        assert_eq!(budget.available(), 0);
        budget.release(25);
        assert_eq!(budget.total(), 10);
    }
}
