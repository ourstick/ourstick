//! Non-secret local state: this device, the sticks it owns, the sticks it reads.
//!
//! Secrets never go through a [`Store`]; they live in a
//! [`SecretStore`](crate::secrets::SecretStore). The schema is described in
//! `docs/schema.md`.
//!
//! All methods block. From async code, call them inside `spawn_blocking`.

mod checks;
mod memory;
pub mod reports;
mod sqlite;
mod types;

use std::path::{Path, PathBuf};

use crate::manifest::FileStamp;
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

pub(crate) use checks::*;
pub use memory::MemoryStore;
pub use reports::{KeepOutcome, KeptReport};
pub use sqlite::{SqliteStore, SCHEMA_VERSION};
pub use types::*;

/// Local state storage.
pub trait Store: Send + Sync {
    fn device(&self) -> Result<Option<Device>, StoreError>;

    /// Records this device. Fails if one exists: the device is never replaced.
    fn create_device(&self, device: &Device) -> Result<(), StoreError>;

    /// Records a new owned stick. Fails if it exists.
    fn add_owned_stick(&self, stick: &OwnedStick) -> Result<(), StoreError>;

    fn owned_stick(&self, stick_id: &StickId) -> Result<Option<OwnedStick>, StoreError>;

    /// All owned sticks, oldest first.
    fn owned_sticks(&self) -> Result<Vec<OwnedStick>, StoreError>;

    /// Records that an owned stick's folder is now at `root` (a drive whose
    /// disk image mounted elsewhere, e.g. `/Volumes/OurStick 1`).
    fn move_owned_stick(&self, stick_id: &StickId, root: &Path) -> Result<(), StoreError>;

    /// Records the drive letter an owned stick was given (Windows), or
    /// forgets it. Fails with [`StoreError::Inconsistent`] unless it is `A`–`Z`.
    fn set_drive_letter(&self, stick_id: &StickId, letter: Option<char>) -> Result<(), StoreError>;

    /// Stores a record the owner just signed. Fails with
    /// [`StoreError::NotNewer`] unless its `seq` is greater than the stored one,
    /// checked and updated atomically.
    /// Saves `record` as the latest of its stick; `partial` says whether its
    /// manifest is partial. Fails unless its `seq` is above every one saved.
    fn save_signed_record(
        &self,
        record: &SignedManifestRecord,
        partial: bool,
    ) -> Result<(), StoreError>;

    /// The files recorded at the last publish of an owned stick, by path.
    fn owned_files(&self, stick_id: &StickId) -> Result<Vec<OwnedFile>, StoreError>;

    /// Each of [`Store::owned_files`], by path, handed to `each` one at a
    /// time instead of all in one list (ADR 0019): `each` stops the walk
    /// with `Break`. The store is held meanwhile: `each` must not wait.
    fn each_owned_file(
        &self,
        stick_id: &StickId,
        each: &mut dyn FnMut(&OwnedFile) -> std::ops::ControlFlow<()>,
    ) -> Result<(), StoreError>;

    /// Records files of an owned stick, replacing the rows of the same paths.
    /// All or nothing: a batch of the catalogue (ADR 0007) is kept as a whole
    /// or not at all.
    fn record_owned_files(&self, stick_id: &StickId, files: &[OwnedFile])
        -> Result<(), StoreError>;

    /// Forgets the recorded files of an owned stick at these paths, so the
    /// next pass reads them again (their record no longer matched them).
    fn forget_owned_files(&self, stick_id: &StickId, paths: &[String]) -> Result<(), StoreError>;

    /// Forgets the recorded files of an owned stick whose path is not in
    /// `paths` (files that left the folder), at the end of a full pass.
    fn keep_only_owned_files(&self, stick_id: &StickId, paths: &[String])
        -> Result<(), StoreError>;

    /// The sticks being removed (hidden from every other read), for the
    /// one place that finishes their removal.
    fn removing_sticks(&self) -> Result<Vec<OwnedStick>, StoreError>;

    /// Marks a stick as being removed: from now on it is hidden from
    /// [`Store::owned_stick`] and [`Store::owned_sticks`].
    fn mark_removing(&self, stick_id: &StickId) -> Result<(), StoreError>;

    /// Deletes a stick being removed, with its files' records, linked
    /// entries and claims. Refused for a stick not marked removing.
    fn remove_owned_stick(&self, stick_id: &StickId) -> Result<(), StoreError>;

    /// Gives a stick a new name (checked by the caller).
    fn rename_owned_stick(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError>;

    /// Whether the user wants the stick shared ("Stop sharing" sets false).
    fn set_sharing_wanted(&self, stick_id: &StickId, wanted: bool) -> Result<(), StoreError>;

    /// Records a macOS stick's disk image.
    fn set_image_path(&self, stick_id: &StickId, path: &Path) -> Result<(), StoreError>;

    /// "Give it a new link" (ADR 0009 §5), in one transaction: `new` (a new
    /// id, the old one's folder and settings) takes the old stick's place
    /// and linked entries; the old stick's row, files' records and claims go,
    /// and its id is retired (only its secrets and cache are left to delete:
    /// [`Store::retired_sticks`]).
    fn replace_owned_stick(
        &self,
        old: &StickId,
        new: &OwnedStick,
        now_ms: u64,
    ) -> Result<(), StoreError>;

    /// Ids retired by a new link whose secrets and cache are not deleted
    /// yet, with the id that replaced each.
    fn retired_sticks(&self) -> Result<Vec<RetiredStick>, StoreError>;

    /// A retired id is cleaned up.
    fn forget_retired(&self, stick_id: &StickId) -> Result<(), StoreError>;

    /// Adds a folder or file shared where it is (ADR 0008). Fails with
    /// [`StoreError::LinkExists`] if the stick has an entry of that name
    /// (exactly; case is the caller's, see `crate::linked`).
    fn add_linked_entry(&self, stick_id: &StickId, entry: &LinkedEntry) -> Result<(), StoreError>;

    /// The stick's linked entries, by name.
    fn linked_entries(&self, stick_id: &StickId) -> Result<Vec<LinkedEntry>, StoreError>;

    /// Stops sharing a linked entry: removes its row, and the recorded files
    /// under its name. The user's files are never touched.
    fn remove_linked_entry(&self, stick_id: &StickId, name: &str) -> Result<(), StoreError>;

    /// A linked entry that was not found is somewhere else now ("Find it…",
    /// ADR 0029): the entry named `entry.name` takes its path and disk; its
    /// recorded files stay, so what did not change is not hashed again.
    /// Fails with [`StoreError::LinkNotFound`], or
    /// [`StoreError::Inconsistent`] if it would change from a folder to a
    /// file, or move to or from the top.
    fn move_linked_entry(&self, stick_id: &StickId, entry: &LinkedEntry) -> Result<(), StoreError>;

    /// Every claim on room, stale ones included (the caller tells them
    /// apart).
    fn space_claims(&self) -> Result<Vec<SpaceClaim>, StoreError>;

    /// Decides and records claims at once, so two programs never both take
    /// the same room: `decide` gets every claim and returns those to record
    /// (each replacing the stick's claim of its kind), or none. One
    /// transaction: all are recorded, or none.
    fn claim_space(
        &self,
        decide: &mut dyn FnMut(&[SpaceClaim]) -> Vec<SpaceClaim>,
    ) -> Result<Vec<SpaceClaim>, StoreError>;

    /// Changes how much a claim holds; nothing if there is none.
    fn update_space_claim(
        &self,
        stick_id: &StickId,
        kind: ClaimKind,
        bytes: u64,
    ) -> Result<(), StoreError>;

    /// Drops a claim; nothing if there is none.
    fn remove_space_claim(&self, stick_id: &StickId, kind: ClaimKind) -> Result<(), StoreError>;

    /// Records a new joined stick, ending any wait for it (ADR 0025) in the
    /// same step. Fails if it exists.
    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError>;

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError>;

    /// All joined sticks, oldest first.
    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError>;

    /// Points a joined stick at another folder, which holds nothing yet:
    /// clears `completed` and the downloaded files.
    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError>;

    /// Changes a received stick's name, program or pause, or marks it as
    /// being forgotten (hidden from the reads above from then on).
    /// [`StoreError::StickNotFound`] if there is no such live stick.
    fn change_joined_stick(&self, id: &StickId, change: &JoinedChange) -> Result<(), StoreError>;

    /// The received sticks being forgotten, oldest first.
    fn forgetting_joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError>;

    /// Deletes a received stick being forgotten: its row, what its folder
    /// was recorded to hold, and its download's claims. Its record floor
    /// stays (ADR 0004).
    fn forget_joined_stick(&self, stick_id: &StickId) -> Result<(), StoreError>;

    /// Records that every file of `version` is in the stick's folder. Fails
    /// with [`StoreError::Inconsistent`] unless a record of at least that
    /// `seq` was accepted.
    fn complete_download(
        &self,
        stick_id: &StickId,
        version: ManifestVersion,
    ) -> Result<(), StoreError>;

    /// The files `get` wrote into a joined stick's current folder, by path.
    fn downloaded_files(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError>;

    /// Records (or replaces) files `get` wrote into the stick's folder, in
    /// order, in one transaction: all of them or none (ADR 0014). Two files
    /// of a stick are never recorded at one place (`saved_as`).
    fn record_downloaded_files(
        &self,
        stick_id: &StickId,
        files: &[DownloadedFile],
    ) -> Result<(), StoreError>;

    /// One file, as [`Store::record_downloaded_files`].
    fn record_downloaded_file(
        &self,
        stick_id: &StickId,
        file: &DownloadedFile,
    ) -> Result<(), StoreError> {
        self.record_downloaded_files(stick_id, std::slice::from_ref(file))
    }

    /// The top-level names of a received stick the user chose not to get
    /// (ADR 0012), sorted; empty: everything is got.
    fn left_out(&self, stick_id: &StickId) -> Result<Vec<String>, StoreError>;

    /// Replaces what the user chose not to get of a received stick (an
    /// empty set: everything), in one go.
    fn set_left_out(&self, stick_id: &StickId, names: &[String]) -> Result<(), StoreError>;

    /// Whether files of a received stick under its top-level `name` (the
    /// file `name`, or anything in the folder `name`) are recorded here.
    fn has_files_under(&self, stick_id: &StickId, name: &str) -> Result<bool, StoreError> {
        let folder = format!("{name}/");
        Ok(self
            .downloaded_files(stick_id)?
            .iter()
            .any(|file| file.path == name || file.path.starts_with(&folder)))
    }

    /// Forgets one recorded file of a received stick (its file is gone, or it
    /// is to be saved at another place); nothing if there is none.
    fn forget_downloaded_file(&self, stick_id: &StickId, path: &str) -> Result<(), StoreError>;

    /// The recorded files saved under another name than their path in the
    /// stick (ADR 0010), by path.
    fn saved_elsewhere(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        Ok(self
            .downloaded_files(stick_id)?
            .into_iter()
            .filter(|file| file.saved_as != file.path)
            .collect())
    }

    /// The highest record accepted for a stick (the `last_seen` of ADR 0004),
    /// whether or not it was joined.
    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError>;

    /// Raises the floor to a record the reader has verified. Fails with
    /// [`StoreError::NotNewer`] unless its `seq` is greater than the stored one
    /// (or none is stored), checked and updated atomically.
    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError>;

    /// Keeps a reader's opened report on an owned stick (ADR 0015, rule 5):
    /// replaces that reader's line if its counter is higher; a new reader
    /// only where [`reports::MAX_READERS`] leaves room (a hidden line, or one
    /// silent for 30 days, makes room); `Forgotten` hides the line, keeping
    /// its counter. Drops lines older than 90 days. `StickNotFound` for a
    /// stick not owned, or being removed.
    fn keep_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
        report: &crate::report::ReaderReport,
        now_ms: u64,
    ) -> Result<KeepOutcome, StoreError>;

    /// The lines the sender sees: not hidden, received in the last 90 days,
    /// the latest first.
    fn reports(&self, stick_id: &StickId, now_ms: u64) -> Result<Vec<KeptReport>, StoreError>;

    /// "Remove from this list": hides one reader's line (its counter kept).
    fn hide_report(
        &self,
        stick_id: &StickId,
        reader: &crate::report::ReaderId,
    ) -> Result<(), StoreError>;

    /// "Clear this list": hides every line of the stick.
    fn hide_reports(&self, stick_id: &StickId) -> Result<(), StoreError>;

    /// The name this device's receiver chose to show senders, if any.
    fn receiver_name(&self) -> Result<Option<String>, StoreError>;

    /// Sets or clears it; refused unless 1-40 characters with none
    /// invisible, or before the device is set up.
    fn set_receiver_name(&self, name: Option<&str>) -> Result<(), StoreError>;

    /// The device's keys were moved to the data protection keychain (ADR
    /// 0021): it is recorded as their store, with whether the login
    /// keychain still holds the old copies. Refused unless the device's
    /// keys were in the login keychain or are already moved.
    fn record_keys_moved(&self, old_keys_left: bool) -> Result<(), StoreError>;

    /// Whether the login keychain still holds copies of moved keys.
    fn old_keys_left(&self) -> Result<bool, StoreError>;

    /// Whether the app explained the Local Network permission on this
    /// device (ADR 0020); false before the device is set up.
    fn local_network_explained(&self) -> Result<bool, StoreError>;

    /// It was explained: true if remembered, false if the device is not set
    /// up yet (to be told again once it is).
    fn set_local_network_explained(&self) -> Result<bool, StoreError>;

    /// The counter of a received stick's next report, kept so it grows
    /// across restarts and a clock set back: the clock's time, or the last
    /// one plus one.
    fn next_report_counter(&self, stick_id: &StickId, now_ms: u64) -> Result<u64, StoreError>;

    /// The app's choices on this device (ADR 0024); the defaults if none
    /// was made yet, even before the device is set up.
    fn settings(&self) -> Result<Settings, StoreError>;

    fn set_settings(&self, settings: &Settings) -> Result<(), StoreError>;

    /// Records a link the app waits for (ADR 0025).
    /// [`StoreError::StickExists`] if it waits already or is received.
    fn add_waiting_link(&self, link: &WaitingLink) -> Result<(), StoreError>;

    /// The links the app waits for, oldest first.
    fn waiting_links(&self) -> Result<Vec<WaitingLink>, StoreError>;

    /// Ends the wait for `stick_id`; nothing if there is none.
    fn remove_waiting_link(&self, stick_id: &StickId) -> Result<(), StoreError>;
}

/// Why the store refused an operation. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("this device is already set up; its keys are never replaced")]
    DeviceExists,
    #[error("this stick is already in the database")]
    StickExists,
    #[error("this stick is not in the database")]
    StickNotFound,
    #[error("the stick already has an entry called {0}")]
    LinkExists(String),
    #[error("the stick has no linked entry called {0}")]
    LinkNotFound(String),
    #[error("seq {seq} is not newer than {stored}, the one already stored")]
    NotNewer { seq: u64, stored: u64 },
    #[error("seq {0} is too large to store")]
    SeqOutOfRange(u64),
    #[error("time {0} ms is too large to store")]
    TimeOutOfRange(u64),
    #[error("{0} {1} is too large to store")]
    ValueOutOfRange(&'static str, u64),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("inconsistent data: {0}")]
    Inconsistent(&'static str),
    #[error("the database was made by a newer version of OurStick (schema {found}, this version knows up to {supported}); update OurStick")]
    NewerDatabase { found: i64, supported: i64 },
    #[error("the database contains invalid data: {0}")]
    Corrupt(String),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests;
