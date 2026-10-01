//! Non-secret local state: this device, the sticks it owns, the sticks it reads.
//!
//! Secrets never go through a [`Store`]; they live in a
//! [`SecretStore`](crate::secrets::SecretStore). The schema is described in
//! `docs/schema.md`.
//!
//! All methods block. From async code, call them inside `spawn_blocking`.

mod checks;
mod memory;
mod sqlite;
mod types;

use std::path::{Path, PathBuf};

use crate::manifest::FileStamp;
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;

pub(crate) use checks::*;
pub use memory::MemoryStore;
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

    /// Records a new joined stick. Fails if it exists.
    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError>;

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError>;

    /// All joined sticks, oldest first.
    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError>;

    /// Points a joined stick at another folder, which holds nothing yet:
    /// clears `completed` and the downloaded files.
    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError>;

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

    /// Records (or replaces) one file `get` wrote into the stick's folder.
    fn record_downloaded_file(
        &self,
        stick_id: &StickId,
        file: &DownloadedFile,
    ) -> Result<(), StoreError>;

    /// The highest record accepted for a stick (the `last_seen` of ADR 0004),
    /// whether or not it was joined.
    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError>;

    /// Raises the floor to a record the reader has verified. Fails with
    /// [`StoreError::NotNewer`] unless its `seq` is greater than the stored one
    /// (or none is stored), checked and updated atomically.
    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError>;
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
