//! [`Store`] in memory, for tests. Enforces the same rules as [`SqliteStore`]
//! (the shared tests in `store/tests.rs` run against both).
//!
//! [`SqliteStore`]: super::SqliteStore

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

mod extra;
mod life;

use super::{
    check_device, check_drive_letter, check_joined_stick, check_owned_file, check_owned_stick,
    check_seq, check_stamp, path_str, ClaimKind, Device, DownloadedFile, JoinedStick, LinkedEntry,
    ManifestVersion, OwnedFile, OwnedStick, RetiredStick, SpaceClaim, Store, StoreError,
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
    linked: BTreeMap<[u8; 32], BTreeMap<String, LinkedEntry>>,
    claims: BTreeMap<([u8; 32], u8), SpaceClaim>,
    /// Owned sticks being removed: hidden from the usual reads.
    removing: std::collections::BTreeSet<[u8; 32]>,
    /// Ids retired by a new link, with when.
    retired: BTreeMap<[u8; 32], (u64, StickId)>,
}

fn claim_key(stick_id: &StickId, kind: ClaimKind) -> ([u8; 32], u8) {
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
    items.sort_by_key(|item| (item.0, item.1));
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
        let same_node = stick.node_id.is_some()
            && state
                .owned
                .values()
                .any(|owned| owned.node_id == stick.node_id);
        if state.owned.contains_key(&key) || same_node {
            return Err(StoreError::StickExists);
        }
        state.owned.insert(key, stick.clone());
        Ok(())
    }

    fn owned_stick(&self, stick_id: &StickId) -> Result<Option<OwnedStick>, StoreError> {
        let state = self.state();
        let key = stick_id.to_bytes();
        if state.removing.contains(&key) {
            return Ok(None);
        }
        Ok(state.owned.get(&key).cloned())
    }

    fn owned_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        Ok(life::listed(self, false))
    }

    fn removing_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        Ok(life::listed(self, true))
    }

    fn mark_removing(&self, stick_id: &StickId) -> Result<(), StoreError> {
        life::mark_removing(self, stick_id)
    }

    fn remove_owned_stick(&self, stick_id: &StickId) -> Result<(), StoreError> {
        life::remove(self, stick_id)
    }

    fn rename_owned_stick(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError> {
        life::change(self, stick_id, |stick| stick.name = name.to_owned())
    }

    fn set_sharing_wanted(&self, stick_id: &StickId, wanted: bool) -> Result<(), StoreError> {
        life::change(self, stick_id, |stick| stick.sharing_wanted = wanted)
    }

    fn set_image_path(&self, stick_id: &StickId, path: &Path) -> Result<(), StoreError> {
        path_str(path)?;
        life::change(self, stick_id, |stick| {
            stick.image_path = Some(path.to_path_buf())
        })
    }

    fn replace_owned_stick(
        &self,
        old: &StickId,
        new: &OwnedStick,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        life::replace(self, old, new, now_ms)
    }

    fn retired_sticks(&self) -> Result<Vec<RetiredStick>, StoreError> {
        life::retired(self)
    }

    fn forget_retired(&self, stick_id: &StickId) -> Result<(), StoreError> {
        self.state().retired.remove(&stick_id.to_bytes());
        Ok(())
    }

    fn move_owned_stick(&self, stick_id: &StickId, root: &Path) -> Result<(), StoreError> {
        path_str(root)?;
        life::change(self, stick_id, |stick| stick.root_path = root.to_path_buf())
    }

    fn set_drive_letter(&self, stick_id: &StickId, letter: Option<char>) -> Result<(), StoreError> {
        check_drive_letter(letter)?;
        life::change(self, stick_id, |stick| stick.drive_letter = letter)
    }

    fn save_signed_record(
        &self,
        record: &SignedManifestRecord,
        partial: bool,
    ) -> Result<(), StoreError> {
        life::save_signed_record(self, record, partial)
    }

    fn owned_files(&self, stick_id: &StickId) -> Result<Vec<OwnedFile>, StoreError> {
        let state = self.state();
        Ok(state
            .owned_files
            .get(&stick_id.to_bytes())
            .map(|files| files.values().cloned().collect())
            .unwrap_or_default())
    }

    fn record_owned_files(
        &self,
        stick_id: &StickId,
        files: &[OwnedFile],
    ) -> Result<(), StoreError> {
        for file in files {
            check_owned_file(file)?;
        }
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if !life::is_live(&state, &key) {
            return Err(StoreError::StickNotFound);
        }
        let recorded = state.owned_files.entry(key).or_default();
        for file in files {
            recorded.insert(file.path.clone(), file.clone());
        }
        Ok(())
    }

    fn forget_owned_files(&self, stick_id: &StickId, paths: &[String]) -> Result<(), StoreError> {
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if !life::is_live(&state, &key) {
            return Err(StoreError::StickNotFound);
        }
        if let Some(recorded) = state.owned_files.get_mut(&key) {
            for path in paths {
                recorded.remove(path);
            }
        }
        Ok(())
    }

    fn keep_only_owned_files(
        &self,
        stick_id: &StickId,
        paths: &[String],
    ) -> Result<(), StoreError> {
        let mut state = self.state();
        let key = stick_id.to_bytes();
        if !life::is_live(&state, &key) {
            return Err(StoreError::StickNotFound);
        }
        let keep: std::collections::HashSet<&String> = paths.iter().collect();
        if let Some(recorded) = state.owned_files.get_mut(&key) {
            recorded.retain(|path, _| keep.contains(path));
        }
        Ok(())
    }

    fn add_linked_entry(&self, stick_id: &StickId, entry: &LinkedEntry) -> Result<(), StoreError> {
        extra::add_linked_entry(self, stick_id, entry)
    }

    fn linked_entries(&self, stick_id: &StickId) -> Result<Vec<LinkedEntry>, StoreError> {
        extra::linked_entries(self, stick_id)
    }

    fn remove_linked_entry(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError> {
        extra::remove_linked_entry(self, stick_id, name)
    }

    fn space_claims(&self) -> Result<Vec<SpaceClaim>, StoreError> {
        extra::space_claims(self)
    }

    fn claim_space(
        &self,
        decide: &mut dyn FnMut(&[SpaceClaim]) -> Option<SpaceClaim>,
    ) -> Result<Option<SpaceClaim>, StoreError> {
        extra::claim_space(self, decide)
    }

    fn update_space_claim(
        &self,
        stick_id: &StickId,
        kind: ClaimKind,
        bytes: u64,
    ) -> Result<(), StoreError> {
        extra::update_space_claim(self, stick_id, kind, bytes)
    }

    fn remove_space_claim(&self, stick_id: &StickId, kind: ClaimKind) -> Result<(), StoreError> {
        extra::remove_space_claim(self, stick_id, kind)
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
