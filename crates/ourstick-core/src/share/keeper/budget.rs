//! The room the owner's suitcases may take (ADR 0007): one budget per
//! program, shared by its sticks, taken from the free space on the cache's
//! disk once, at start (suitcases shrink what is free).

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::share::ShareError;

/// The budget is this share of the free space…
const SHARE_OF_FREE: u64 = 10;
/// …at least this…
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

    /// 10 % of the free space on the disk of `cache_dir`, 2 to 40 GB. Fails
    /// with [`ShareError::NoRoom`] with less than 2 GB free. Blocking.
    pub fn for_cache(cache_dir: &Path) -> Result<Self, ShareError> {
        let free = fs4::available_space(cache_dir).map_err(|source| ShareError::Io {
            path: cache_dir.to_path_buf(),
            source,
        })?;
        Ok(Self::new(budget_for_free_space(free)?))
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

fn budget_for_free_space(free: u64) -> Result<u64, ShareError> {
    if free < MIN_BUDGET {
        return Err(ShareError::NoRoom { free });
    }
    Ok((free / SHARE_OF_FREE).clamp(MIN_BUDGET, MAX_BUDGET))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn the_budget_is_a_tenth_of_the_free_space_within_bounds() {
        assert_eq!(budget_for_free_space(161 * GB).unwrap(), 161 * GB / 10);
        assert_eq!(budget_for_free_space(5 * GB).unwrap(), MIN_BUDGET);
        assert_eq!(budget_for_free_space(483 * GB).unwrap(), MAX_BUDGET);
        assert!(matches!(
            budget_for_free_space(GB),
            Err(ShareError::NoRoom { free }) if free == GB
        ));
    }

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

    #[test]
    fn the_cache_disk_has_a_budget() {
        let dir = tempfile::tempdir().unwrap();
        match Budget::for_cache(dir.path()) {
            Ok(budget) => assert!((MIN_BUDGET..=MAX_BUDGET).contains(&budget.total())),
            Err(ShareError::NoRoom { .. }) => {}
            Err(error) => panic!("{error}"),
        }
    }
}
