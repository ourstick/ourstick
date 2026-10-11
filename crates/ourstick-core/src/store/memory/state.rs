//! The memory store's state: what the database's tables hold.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use super::reports;
use crate::stick_id::StickId;
use crate::store::{
    ClaimKind, Device, DownloadedFile, JoinedStick, LinkedEntry, ManifestVersion, OwnedFile,
    OwnedStick, SpaceClaim,
};

#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) device: Option<Device>,
    pub(super) owned: BTreeMap<[u8; 32], OwnedStick>,
    pub(super) joined: BTreeMap<[u8; 32], JoinedStick>,
    pub(super) floors: BTreeMap<[u8; 32], ManifestVersion>,
    pub(super) owned_files: BTreeMap<[u8; 32], BTreeMap<String, OwnedFile>>,
    pub(super) downloaded_files: BTreeMap<[u8; 32], BTreeMap<String, DownloadedFile>>,
    /// What the user chose not to get of each received stick (ADR 0012).
    pub(super) left_out: BTreeMap<[u8; 32], std::collections::BTreeSet<String>>,
    pub(super) linked: BTreeMap<[u8; 32], BTreeMap<String, LinkedEntry>>,
    pub(super) claims: BTreeMap<([u8; 32], u8), SpaceClaim>,
    /// Owned sticks being removed: hidden from the usual reads.
    pub(super) removing: std::collections::BTreeSet<[u8; 32]>,
    /// Received sticks being forgotten: hidden from the usual reads.
    pub(super) forgetting: std::collections::BTreeSet<[u8; 32]>,
    /// Ids retired by a new link, with when.
    pub(super) retired: BTreeMap<[u8; 32], (u64, StickId)>,
    /// Readers' latest reports on owned sticks (ADR 0015).
    pub(super) reports: BTreeMap<[u8; 32], BTreeMap<crate::report::ReaderId, reports::Line>>,
    pub(super) receiver_name: Option<String>,
    pub(super) local_network_explained: bool,
    pub(super) old_keys_left: bool,
    pub(super) settings: crate::store::Settings,
    pub(super) report_counters: BTreeMap<[u8; 32], u64>,
}

pub(super) fn claim_key(stick_id: &StickId, kind: ClaimKind) -> ([u8; 32], u8) {
    (stick_id.to_bytes(), kind as u8)
}

/// A [`Store`] that forgets everything when dropped.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        // Every operation checks before it changes anything, so a panic while
        // holding the lock leaves the state consistent.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Same order as the SQLite store: oldest first, then by stick id.
pub(super) fn sorted<T: Clone>(items: impl Iterator<Item = (u64, [u8; 32], T)>) -> Vec<T> {
    let mut items: Vec<_> = items.collect();
    items.sort_by_key(|item| (item.0, item.1));
    items.into_iter().map(|(_, _, item)| item).collect()
}
