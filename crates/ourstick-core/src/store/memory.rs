//! [`Store`] in memory, for tests. Enforces the same rules as [`SqliteStore`]
//! (the shared tests in `store/tests.rs` run against both).
//!
//! [`SqliteStore`]: super::SqliteStore

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use super::{
    check_device, check_joined_stick, check_owned_file, check_owned_stick, check_seq, check_stamp,
    path_str, Device, DownloadedFile, JoinedStick, ManifestVersion, OwnedFile, OwnedStick, Store,
    StoreError,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

#[derive(Debug, Default)]
struct State {
    device: Option<Device>,
    owned: BTreeMap<[u8; 32], OwnedStick>,
    joined: BTreeMap<[u8; 32], JoinedStick>,
    floors: BTreeMap<[u8; 32], ManifestVersion>,
    owned_files: BTreeMap<[u8; 32], BTreeMap<String, OwnedFile>>,
    downloaded_files: BTreeMap<[u8; 32], BTreeMap<String, DownloadedFile>>,
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

    fn owned_files(&self, stick_id: &StickId) -> Result<Vec<OwnedFile>, StoreError> {
        let state = self.state();
        Ok(state
            .owned_files
            .get(&stick_id.to_bytes())
            .map(|files| files.values().cloned().collect())
            .unwrap_or_default())
    }

    fn replace_owned_files(
        &self,
        stick_id: &StickId,
        files: &[OwnedFile],
    ) -> Result<(), StoreError> {
        for file in files {
            check_owned_file(file)?;
        }
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if !state.owned.contains_key(&key) {
            return Err(StoreError::StickNotFound);
        }
        let files = files
            .iter()
            .map(|file| (file.path.clone(), file.clone()))
            .collect();
        state.owned_files.insert(key, files);
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

    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError> {
        path_str(output_dir)?;
        let mut state = self.state();
        let stick = state
            .joined
            .get_mut(&stick_id.to_bytes())
            .ok_or(StoreError::StickNotFound)?;
        stick.output_dir = output_dir.to_path_buf();
        stick.completed = None;
        state.downloaded_files.remove(&stick_id.to_bytes());
        Ok(())
    }

    fn complete_download(
        &self,
        stick_id: &StickId,
        version: ManifestVersion,
    ) -> Result<(), StoreError> {
        check_seq(version.seq)?;
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if state
            .floors
            .get(&key)
            .is_none_or(|floor| floor.seq < version.seq)
        {
            return Err(StoreError::Inconsistent(
                "a download completed for a record that was never accepted",
            ));
        }
        let stick = state
            .joined
            .get_mut(&key)
            .ok_or(StoreError::StickNotFound)?;
        stick.completed = Some(version);
        Ok(())
    }

    fn downloaded_files(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        let state = self.state();
        Ok(state
            .downloaded_files
            .get(&stick_id.to_bytes())
            .map(|files| files.values().cloned().collect())
            .unwrap_or_default())
    }

    fn record_downloaded_file(
        &self,
        stick_id: &StickId,
        file: &DownloadedFile,
    ) -> Result<(), StoreError> {
        check_stamp(&file.stamp)?;
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if !state.joined.contains_key(&key) {
            return Err(StoreError::StickNotFound);
        }
        state
            .downloaded_files
            .entry(key)
            .or_default()
            .insert(file.path.clone(), file.clone());
        Ok(())
    }

    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError> {
        Ok(self.state().floors.get(&stick_id.to_bytes()).copied())
    }

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        check_seq(record.seq)?;
        let mut state = self.state();
        let key = record.stick_id.to_bytes();
        if let Some(floor) = state.floors.get(&key) {
            if record.seq <= floor.seq {
                return Err(StoreError::NotNewer {
                    seq: record.seq,
                    stored: floor.seq,
                });
            }
        }
        state.floors.insert(
            key,
            ManifestVersion {
                seq: record.seq,
                hash: record.manifest_hash,
            },
        );
        Ok(())
    }
}
