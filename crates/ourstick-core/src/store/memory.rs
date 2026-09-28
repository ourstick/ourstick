//! [`Store`] in memory, for tests. Enforces the same rules as [`SqliteStore`]
//! (the shared tests in `store/tests.rs` run against both).
//!
//! [`SqliteStore`]: super::SqliteStore

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use super::{
    check_device, check_joined_stick, check_owned_stick, check_seq, CurrentManifest, Device,
    JoinedStick, OwnedStick, Store, StoreError,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

#[derive(Debug, Default)]
struct State {
    device: Option<Device>,
    owned: BTreeMap<[u8; 32], OwnedStick>,
    joined: BTreeMap<[u8; 32], JoinedStick>,
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

    fn state(&self) -> MutexGuard<'_, State> {
        // Every operation checks before it changes anything, so a panic while
        // holding the lock leaves the state consistent.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Same order as the SQLite store: oldest first, then by stick id.
fn sorted<T: Clone>(items: impl Iterator<Item = (u64, [u8; 32], T)>) -> Vec<T> {
    let mut items: Vec<_> = items.collect();
    items.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    items.into_iter().map(|(_, _, item)| item).collect()
}

impl Store for MemoryStore {
    fn device(&self) -> Result<Option<Device>, StoreError> {
        Ok(self.state().device.clone())
    }

    fn create_device(&self, device: &Device) -> Result<(), StoreError> {
        check_device(device)?;
        let mut state = self.state();
        if state.device.is_some() {
            return Err(StoreError::DeviceExists);
        }
        state.device = Some(device.clone());
        Ok(())
    }

    fn add_owned_stick(&self, stick: &OwnedStick) -> Result<(), StoreError> {
        check_owned_stick(stick)?;
        let mut state = self.state();
        let key = stick.stick_id.to_bytes();
        if state.owned.contains_key(&key) {
            return Err(StoreError::StickExists);
        }
        state.owned.insert(key, stick.clone());
        Ok(())
    }

    fn owned_stick(&self, stick_id: &StickId) -> Result<Option<OwnedStick>, StoreError> {
        Ok(self.state().owned.get(&stick_id.to_bytes()).cloned())
    }

    fn owned_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        let state = self.state();
        Ok(sorted(state.owned.iter().map(|(key, stick)| {
            (stick.created_at_ms, *key, stick.clone())
        })))
    }

    fn save_signed_record(&self, record: &SignedManifestRecord) -> Result<(), StoreError> {
        let content = record.unverified_record();
        check_seq(content.seq)?;
        let mut state = self.state();
        let stick = state
            .owned
            .get_mut(&content.stick_id.to_bytes())
            .ok_or(StoreError::StickNotFound)?;
        if content.seq <= stick.last_signed_seq {
            return Err(StoreError::NotNewer {
                seq: content.seq,
                stored: stick.last_signed_seq,
            });
        }
        stick.last_signed_seq = content.seq;
        stick.latest_record = Some(record.clone());
        Ok(())
    }

    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError> {
        check_joined_stick(stick)?;
        let mut state = self.state();
        let key = stick.stick_id.to_bytes();
        if state.joined.contains_key(&key) {
            return Err(StoreError::StickExists);
        }
        state.joined.insert(key, stick.clone());
        Ok(())
    }

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError> {
        Ok(self.state().joined.get(&stick_id.to_bytes()).cloned())
    }

    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        let state = self.state();
        Ok(sorted(state.joined.iter().map(|(key, stick)| {
            (stick.joined_at_ms, *key, stick.clone())
        })))
    }

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        check_seq(record.seq)?;
        let mut state = self.state();
        let stick = state
            .joined
            .get_mut(&record.stick_id.to_bytes())
            .ok_or(StoreError::StickNotFound)?;
        if let Some(current) = &stick.current_manifest {
            if record.seq <= current.seq {
                return Err(StoreError::NotNewer {
                    seq: record.seq,
                    stored: current.seq,
                });
            }
        }
        stick.current_manifest = Some(CurrentManifest {
            seq: record.seq,
            hash: record.manifest_hash,
        });
        Ok(())
    }
}
