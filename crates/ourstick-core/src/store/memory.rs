//! [`Store`] in memory, for tests. Enforces the same rules as [`SqliteStore`]
//! (the shared tests in `store/tests.rs` run against both).
//!
//! [`SqliteStore`]: super::SqliteStore

use std::path::Path;

mod extra;
mod joined;
mod life;
mod reports;
mod state;

pub use state::MemoryStore;
use state::{claim_key, sorted, State};

use super::{
    check_device, check_drive_letter, check_owned_file, check_owned_stick, path_str, ClaimKind,
    Device, DownloadedFile, JoinedChange, JoinedStick, LinkedEntry, ManifestVersion, OwnedFile,
    OwnedStick, RetiredStick, Settings, SpaceClaim, Store, StoreError,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

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

    fn each_owned_file(
        &self,
        stick_id: &StickId,
        each: &mut dyn FnMut(&OwnedFile) -> std::ops::ControlFlow<()>,
    ) -> Result<(), StoreError> {
        let state = self.state();
        for file in state
            .owned_files
            .get(&stick_id.to_bytes())
            .into_iter()
            .flat_map(|files| files.values())
        {
            if each(file).is_break() {
                break;
            }
        }
        Ok(())
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
        decide: &mut dyn FnMut(&[SpaceClaim]) -> Vec<SpaceClaim>,
    ) -> Result<Vec<SpaceClaim>, StoreError> {
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
        joined::add(self, stick)
    }

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError> {
        joined::get(self, stick_id)
    }

    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        joined::list(self, false)
    }

    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError> {
        joined::move_to(self, stick_id, output_dir)
    }

    fn change_joined_stick(&self, id: &StickId, change: &JoinedChange) -> Result<(), StoreError> {
        joined::change(self, id, change)
    }

    fn forgetting_joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        joined::list(self, true)
    }

    fn forget_joined_stick(&self, stick_id: &StickId) -> Result<(), StoreError> {
        joined::forget(self, stick_id)
    }

    fn complete_download(
        &self,
        stick_id: &StickId,
        version: ManifestVersion,
    ) -> Result<(), StoreError> {
        joined::complete(self, stick_id, version)
    }

    fn downloaded_files(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        let state = self.state();
        Ok(state
            .downloaded_files
            .get(&stick_id.to_bytes())
            .map(|files| files.values().cloned().collect())
            .unwrap_or_default())
    }

    fn record_downloaded_files(
        &self,
        stick_id: &StickId,
        files: &[DownloadedFile],
    ) -> Result<(), StoreError> {
        joined::record_files(self, stick_id, files)
    }

    fn left_out(&self, stick_id: &StickId) -> Result<Vec<String>, StoreError> {
        joined::left_out(self, stick_id)
    }

    fn set_left_out(&self, stick_id: &StickId, names: &[String]) -> Result<(), StoreError> {
        joined::set_left_out(self, stick_id, names)
    }

    fn forget_downloaded_file(&self, stick_id: &StickId, path: &str) -> Result<(), StoreError> {
        if let Some(files) = self.state().downloaded_files.get_mut(&stick_id.to_bytes()) {
            files.remove(path);
        }
        Ok(())
    }

    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError> {
        Ok(self.state().floors.get(&stick_id.to_bytes()).copied())
    }

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        joined::accept_record(self, record)
    }

    fn keep_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
        report: &crate::report::ReaderReport,
        now_ms: u64,
    ) -> Result<super::KeepOutcome, StoreError> {
        reports::keep(self, stick_id, reader, report, now_ms)
    }

    fn reports(
        &self,
        stick_id: &StickId,
        now_ms: u64,
    ) -> Result<Vec<super::KeptReport>, StoreError> {
        reports::reports(self, stick_id, now_ms)
    }

    fn hide_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
    ) -> Result<(), StoreError> {
        reports::hide(self, stick_id, Some(reader))
    }

    fn hide_reports(&self, stick_id: &StickId) -> Result<(), StoreError> {
        reports::hide(self, stick_id, None)
    }

    fn receiver_name(&self) -> Result<Option<String>, StoreError> {
        Ok(self.state().receiver_name.clone())
    }

    fn set_receiver_name(&self, name: Option<&str>) -> Result<(), StoreError> {
        reports::set_receiver_name(self, name)
    }

    fn record_keys_moved(&self, old_keys_left: bool) -> Result<(), StoreError> {
        reports::record_keys_moved(self, old_keys_left)
    }

    fn old_keys_left(&self) -> Result<bool, StoreError> {
        Ok(self.state().old_keys_left)
    }

    fn local_network_explained(&self) -> Result<bool, StoreError> {
        let state = self.state();
        Ok(state.device.is_some() && state.local_network_explained)
    }

    fn set_local_network_explained(&self) -> Result<bool, StoreError> {
        let mut state = self.state();
        state.local_network_explained = state.device.is_some();
        Ok(state.local_network_explained)
    }

    fn next_report_counter(&self, stick_id: &StickId, now_ms: u64) -> Result<u64, StoreError> {
        reports::next_report_counter(self, stick_id, now_ms)
    }

    fn settings(&self) -> Result<Settings, StoreError> {
        Ok(self.state().settings.clone())
    }

    fn set_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        self.state().settings = settings.clone();
        Ok(())
    }
}
