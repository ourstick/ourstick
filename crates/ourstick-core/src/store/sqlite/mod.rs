//! [`Store`] on SQLite, with migrations from `crates/ourstick-core/migrations/`.

mod device;
mod files;
mod joined;
mod life;
mod linked;
mod migrations;
mod open;
mod owned;
mod received;
mod reports;
mod rows;
mod space;
mod waiting;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

#[cfg(test)]
use migrations::MIGRATIONS;
pub use migrations::SCHEMA_VERSION;
use rows::{blob32, is_constraint_violation, seq_i64, to_u64, OwnedRow, OWNED_COLUMNS};

use super::{
    check_owned_stick, ClaimKind, Device, DownloadedFile, JoinedChange, JoinedStick, KeepOutcome,
    KeptReport, LinkedEntry, ManifestVersion, OwnedFile, OwnedStick, RetiredStick, Settings,
    SpaceClaim, Store, StoreError, WaitingLink,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

/// A [`Store`] in a SQLite database.
#[derive(Debug)]
pub struct SqliteStore {
    connection: Mutex<Connection>,
}

impl Store for SqliteStore {
    fn device(&self) -> Result<Option<Device>, StoreError> {
        device::device(&self.connection())
    }

    fn create_device(&self, device: &Device) -> Result<(), StoreError> {
        device::create_device(&self.connection(), device)
    }

    fn add_owned_stick(&self, stick: &OwnedStick) -> Result<(), StoreError> {
        check_owned_stick(stick)?;
        match owned::insert_owned(&self.connection(), stick) {
            Err(StoreError::Sqlite(error)) if is_constraint_violation(&error) => {
                Err(StoreError::StickExists)
            }
            other => other,
        }
    }

    fn owned_stick(&self, stick_id: &StickId) -> Result<Option<OwnedStick>, StoreError> {
        owned::stick(&self.connection(), stick_id)
    }

    fn owned_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {OWNED_COLUMNS} FROM owned_sticks WHERE removing = 0
             ORDER BY created_at_ms, stick_id"
        ))?;
        let rows = statement.query_map([], OwnedRow::read)?;
        rows.map(|row| row?.into_stick()).collect()
    }

    fn move_owned_stick(&self, stick_id: &StickId, root: &Path) -> Result<(), StoreError> {
        owned::move_owned_stick(&self.connection(), stick_id, root)
    }

    fn set_drive_letter(&self, stick_id: &StickId, letter: Option<char>) -> Result<(), StoreError> {
        owned::set_drive_letter(&self.connection(), stick_id, letter)
    }

    fn save_signed_record(
        &self,
        record: &SignedManifestRecord,
        partial: bool,
    ) -> Result<(), StoreError> {
        owned::save_signed_record(&self.connection(), record, partial)
    }

    fn owned_files(&self, stick_id: &StickId) -> Result<Vec<OwnedFile>, StoreError> {
        files::owned_files(&self.connection(), stick_id)
    }

    fn each_owned_file(
        &self,
        stick_id: &StickId,
        each: &mut dyn FnMut(&OwnedFile) -> std::ops::ControlFlow<()>,
    ) -> Result<(), StoreError> {
        files::each_owned_file(&self.connection(), stick_id, each)
    }

    fn record_owned_files(
        &self,
        stick_id: &StickId,
        files: &[OwnedFile],
    ) -> Result<(), StoreError> {
        files::record_owned_files(&mut self.connection(), stick_id, files)
    }

    fn forget_owned_files(&self, stick_id: &StickId, paths: &[String]) -> Result<(), StoreError> {
        files::forget_owned_files(&mut self.connection(), stick_id, paths)
    }

    fn keep_only_owned_files(
        &self,
        stick_id: &StickId,
        paths: &[String],
    ) -> Result<(), StoreError> {
        files::keep_only_owned_files(&mut self.connection(), stick_id, paths)
    }

    fn removing_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        life::removing(&self.connection())
    }

    fn mark_removing(&self, stick_id: &StickId) -> Result<(), StoreError> {
        life::mark_removing(&self.connection(), stick_id)
    }

    fn remove_owned_stick(&self, stick_id: &StickId) -> Result<(), StoreError> {
        life::remove(&mut self.connection(), stick_id)
    }

    fn rename_owned_stick(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError> {
        life::rename(&self.connection(), stick_id, name)
    }

    fn set_sharing_wanted(&self, stick_id: &StickId, wanted: bool) -> Result<(), StoreError> {
        life::set_sharing_wanted(&self.connection(), stick_id, wanted)
    }

    fn set_image_path(&self, stick_id: &StickId, path: &Path) -> Result<(), StoreError> {
        life::set_image_path(&self.connection(), stick_id, path)
    }

    fn replace_owned_stick(
        &self,
        old: &StickId,
        new: &OwnedStick,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        life::replace(&mut self.connection(), old, new, now_ms)
    }

    fn retired_sticks(&self) -> Result<Vec<RetiredStick>, StoreError> {
        life::retired(&self.connection())
    }

    fn forget_retired(&self, stick_id: &StickId) -> Result<(), StoreError> {
        life::forget_retired(&self.connection(), stick_id)
    }

    fn add_linked_entry(&self, stick_id: &StickId, entry: &LinkedEntry) -> Result<(), StoreError> {
        linked::add(&self.connection(), stick_id, entry)
    }

    fn linked_entries(&self, stick_id: &StickId) -> Result<Vec<LinkedEntry>, StoreError> {
        linked::entries(&self.connection(), stick_id)
    }

    fn remove_linked_entry(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError> {
        linked::remove(&mut self.connection(), stick_id, name)
    }

    fn move_linked_entry(&self, stick_id: &StickId, entry: &LinkedEntry) -> Result<(), StoreError> {
        linked::move_to(&mut self.connection(), stick_id, entry)
    }

    fn space_claims(&self) -> Result<Vec<SpaceClaim>, StoreError> {
        space::claims(&self.connection())
    }

    fn claim_space(
        &self,
        decide: &mut dyn FnMut(&[SpaceClaim]) -> Vec<SpaceClaim>,
    ) -> Result<Vec<SpaceClaim>, StoreError> {
        space::claim(&mut self.connection(), decide)
    }

    fn update_space_claim(
        &self,
        stick_id: &StickId,
        kind: ClaimKind,
        bytes: u64,
    ) -> Result<(), StoreError> {
        space::update(&self.connection(), stick_id, kind, bytes)
    }

    fn remove_space_claim(&self, stick_id: &StickId, kind: ClaimKind) -> Result<(), StoreError> {
        space::remove(&self.connection(), stick_id, kind)
    }

    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError> {
        joined::add_joined_stick(&mut self.connection(), stick)
    }

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError> {
        joined::joined_stick(&self.connection(), stick_id)
    }

    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        joined::joined_sticks(&self.connection())
    }

    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError> {
        joined::move_joined_stick(&mut self.connection(), stick_id, output_dir)
    }

    fn change_joined_stick(&self, id: &StickId, change: &JoinedChange) -> Result<(), StoreError> {
        received::change(&self.connection(), id, change)
    }

    fn forgetting_joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        received::forgetting(&self.connection())
    }

    fn forget_joined_stick(&self, stick_id: &StickId) -> Result<(), StoreError> {
        received::forget(&mut self.connection(), stick_id)
    }

    fn complete_download(
        &self,
        stick_id: &StickId,
        version: ManifestVersion,
    ) -> Result<(), StoreError> {
        joined::complete_download(&mut self.connection(), stick_id, version)
    }

    fn forget_downloaded_file(&self, stick_id: &StickId, path: &str) -> Result<(), StoreError> {
        files::forget_downloaded_file(&self.connection(), stick_id, path)
    }

    fn saved_elsewhere(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        files::saved_elsewhere(&self.connection(), stick_id)
    }

    fn left_out(&self, stick_id: &StickId) -> Result<Vec<String>, StoreError> {
        received::left_out(&self.connection(), stick_id)
    }

    fn set_left_out(&self, stick_id: &StickId, names: &[String]) -> Result<(), StoreError> {
        received::set_left_out(&mut self.connection(), stick_id, names)
    }

    fn has_files_under(&self, stick_id: &StickId, name: &str) -> Result<bool, StoreError> {
        files::has_files_under(&self.connection(), stick_id, name)
    }

    fn downloaded_files(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        files::downloaded_files(&self.connection(), stick_id)
    }

    fn record_downloaded_files(
        &self,
        stick_id: &StickId,
        files: &[DownloadedFile],
    ) -> Result<(), StoreError> {
        files::record_downloaded_files(&mut self.connection(), stick_id, files)
    }

    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError> {
        let row: Option<(i64, Vec<u8>)> = self
            .connection()
            .query_row(
                "SELECT highest_seq, manifest_hash FROM record_floors WHERE stick_id = ?1",
                [stick_id.to_bytes()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(seq, hash)| {
            Ok(ManifestVersion {
                seq: to_u64(seq, "highest_seq")?,
                hash: blob32(hash, "manifest_hash")?,
            })
        })
        .transpose()
    }

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        let seq = seq_i64(record.seq)?;
        let stick_id = record.stick_id.to_bytes();
        let connection = self.connection();
        // One statement: insert, or raise an existing floor only if higher.
        let changed = connection.execute(
            "INSERT INTO record_floors (stick_id, highest_seq, manifest_hash)
             VALUES (?3, ?1, ?2)
             ON CONFLICT (stick_id) DO UPDATE
             SET highest_seq = excluded.highest_seq, manifest_hash = excluded.manifest_hash
             WHERE record_floors.highest_seq < excluded.highest_seq",
            params![seq, record.manifest_hash.to_vec(), stick_id],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let stored: i64 = connection.query_row(
            "SELECT highest_seq FROM record_floors WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )?;
        Err(StoreError::NotNewer {
            seq: record.seq,
            stored: to_u64(stored, "highest_seq")?,
        })
    }

    fn keep_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
        report: &crate::report::ReaderReport,
        now_ms: u64,
    ) -> Result<KeepOutcome, StoreError> {
        reports::keep_report(&mut self.connection(), stick_id, reader, report, now_ms)
    }

    fn reports(&self, stick_id: &StickId, now_ms: u64) -> Result<Vec<KeptReport>, StoreError> {
        reports::reports(&self.connection(), stick_id, now_ms)
    }

    fn hide_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
    ) -> Result<(), StoreError> {
        reports::hide(&self.connection(), stick_id, Some(reader))
    }

    fn hide_reports(&self, stick_id: &StickId) -> Result<(), StoreError> {
        reports::hide(&self.connection(), stick_id, None)
    }

    fn receiver_name(&self) -> Result<Option<String>, StoreError> {
        reports::receiver_name(&self.connection())
    }

    fn set_receiver_name(&self, name: Option<&str>) -> Result<(), StoreError> {
        reports::set_receiver_name(&self.connection(), name)
    }

    fn record_keys_moved(&self, old_keys_left: bool) -> Result<(), StoreError> {
        device::record_keys_moved(&self.connection(), old_keys_left)
    }

    fn old_keys_left(&self) -> Result<bool, StoreError> {
        device::flag(&self.connection(), "old_keys_left")
    }

    fn local_network_explained(&self) -> Result<bool, StoreError> {
        device::flag(&self.connection(), "local_network_explained")
    }

    fn set_local_network_explained(&self) -> Result<bool, StoreError> {
        device::set_local_network_explained(&self.connection())
    }

    fn next_report_counter(&self, stick_id: &StickId, now_ms: u64) -> Result<u64, StoreError> {
        reports::next_report_counter(&mut self.connection(), stick_id, now_ms)
    }

    fn settings(&self) -> Result<Settings, StoreError> {
        device::settings(&self.connection())
    }

    fn set_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        device::set_settings(&self.connection(), settings)
    }

    fn add_waiting_link(&self, link: &WaitingLink) -> Result<(), StoreError> {
        waiting::add_waiting_link(&mut self.connection(), link)
    }

    fn waiting_links(&self) -> Result<Vec<WaitingLink>, StoreError> {
        waiting::waiting_links(&self.connection())
    }

    fn remove_waiting_link(&self, stick_id: &StickId) -> Result<(), StoreError> {
        waiting::remove_waiting_link(&self.connection(), stick_id)
    }
}

#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod tests;
