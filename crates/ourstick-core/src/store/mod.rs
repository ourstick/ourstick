//! Non-secret local state: this device, the sticks it owns, the sticks it reads.
//!
//! Secrets never go through a [`Store`]; they live in a
//! [`SecretStore`](crate::secrets::SecretStore). The schema is described in
//! `docs/schema.md`.
//!
//! All methods block. From async code, call them inside `spawn_blocking`.

mod memory;
mod sqlite;

use std::path::{Path, PathBuf};

use crate::cipher::CipherId;
use crate::manifest::FileStamp;
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;
use crate::ticket::NodeId;

pub use memory::MemoryStore;
pub use sqlite::{SqliteStore, SCHEMA_VERSION};

/// Where this device keeps its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretBackend {
    /// The freedesktop Secret Service (Linux).
    SecretService,
    /// The login keychain (macOS).
    Keychain,
    /// The user's Credential Manager (Windows).
    CredentialManager,
    /// A key file the user chose.
    KeyFile(PathBuf),
}

/// This device. Created once, never replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub node_id: NodeId,
    pub secret_backend: SecretBackend,
    pub created_at_ms: u64,
}

/// A stick this device owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedStick {
    pub stick_id: StickId,
    pub name: String,
    pub root_path: PathBuf,
    pub cipher: CipherId,
    /// Highest `seq` ever signed; 0 if none.
    pub last_signed_seq: u64,
    /// The latest signed record, if any. Its `seq` equals `last_signed_seq`.
    pub latest_record: Option<SignedManifestRecord>,
    /// Whether `latest_record` is of a partial manifest (ADR 0007): no
    /// complete version was published since.
    pub latest_partial: bool,
    pub created_at_ms: u64,
    /// `Some(size in bytes)` for a drive (the app's stick): its folder must
    /// carry the drive marker and may hold at most this much. `None` for a
    /// plain shared folder.
    pub drive_size: Option<u64>,
    /// Windows: the drive letter the stick was last given (`A`–`Z`), asked
    /// for again after a restart. `None` elsewhere.
    pub drive_letter: Option<char>,
}

impl OwnedStick {
    /// A stick that has not signed anything yet.
    pub fn new(
        stick_id: StickId,
        name: impl Into<String>,
        root_path: impl Into<PathBuf>,
        cipher: CipherId,
        created_at_ms: u64,
    ) -> Self {
        Self {
            stick_id,
            name: name.into(),
            root_path: root_path.into(),
            cipher,
            last_signed_seq: 0,
            latest_record: None,
            latest_partial: false,
            created_at_ms,
            drive_size: None,
            drive_letter: None,
        }
    }
}

/// One version of a stick's manifest: the `seq` and manifest hash of a signed
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestVersion {
    pub seq: u64,
    pub hash: [u8; 32],
}

/// A stick this device reads, and what its folder holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedStick {
    pub stick_id: StickId,
    /// The folder of the latest `get`; another folder replaces it.
    pub output_dir: PathBuf,
    /// The version whose files are all in `output_dir`; `None` until a
    /// download finishes there.
    pub completed: Option<ManifestVersion>,
    pub joined_at_ms: u64,
}

impl JoinedStick {
    /// A stick joined from a ticket, before any record was accepted.
    pub fn new(stick_id: StickId, output_dir: impl Into<PathBuf>, joined_at_ms: u64) -> Self {
        Self {
            stick_id,
            output_dir: output_dir.into(),
            completed: None,
            joined_at_ms,
        }
    }
}

/// What the owner last encrypted for one file of a stick: the file's stamp
/// then, and the blob made from it. Lets an unchanged file skip encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFile {
    /// Path relative to the stick root, as in the manifest.
    pub path: String,
    pub stamp: FileStamp,
    pub blob_hash: [u8; 32],
    /// BLAKE3 of the file's plaintext then (the nonce prefix, ADR 0003); a
    /// hint only. `None` for files recorded before ADR 0007.
    pub plaintext_hash: Option<[u8; 32]>,
}

/// A file `get` wrote into a joined stick's folder: the blob it came from,
/// and the file's stamp right after it was written. A file whose stamp is no
/// longer this was changed locally and is not overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedFile {
    /// Path relative to the folder, as in the manifest.
    pub path: String,
    pub stamp: FileStamp,
    pub blob_hash: [u8; 32],
}

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

/// Largest `seq` a store accepts (SQLite INTEGER is signed 64-bit).
pub(crate) const MAX_STORED_SEQ: u64 = i64::MAX as u64;

pub(crate) fn check_seq(seq: u64) -> Result<(), StoreError> {
    if seq > MAX_STORED_SEQ {
        Err(StoreError::SeqOutOfRange(seq))
    } else {
        Ok(())
    }
}

pub(crate) fn check_time(ms: u64) -> Result<(), StoreError> {
    if ms > MAX_STORED_SEQ {
        Err(StoreError::TimeOutOfRange(ms))
    } else {
        Ok(())
    }
}

pub(crate) fn path_str(path: &Path) -> Result<&str, StoreError> {
    path.to_str()
        .ok_or_else(|| StoreError::NonUtf8Path(path.to_path_buf()))
}

/// Checks the rules every store enforces before writing an owned stick.
/// A drive letter is one of `A`–`Z`.
pub(crate) fn check_drive_letter(letter: Option<char>) -> Result<(), StoreError> {
    match letter {
        Some(letter) if !letter.is_ascii_uppercase() => {
            Err(StoreError::Inconsistent("a drive letter must be A to Z"))
        }
        _ => Ok(()),
    }
}

pub(crate) fn check_owned_stick(stick: &OwnedStick) -> Result<(), StoreError> {
    check_seq(stick.last_signed_seq)?;
    check_drive_letter(stick.drive_letter)?;
    match stick.drive_size {
        Some(0) => return Err(StoreError::Inconsistent("a drive of size 0")),
        Some(size) if size > MAX_STORED_SEQ => {
            return Err(StoreError::ValueOutOfRange("drive size", size))
        }
        _ => {}
    }
    check_time(stick.created_at_ms)?;
    path_str(&stick.root_path)?;
    match &stick.latest_record {
        None if stick.last_signed_seq != 0 => Err(StoreError::Inconsistent(
            "a stick that has signed must keep its latest record",
        )),
        Some(record) => {
            let content = record.unverified_record();
            if content.stick_id != stick.stick_id {
                Err(StoreError::Inconsistent("record belongs to another stick"))
            } else if content.seq != stick.last_signed_seq {
                Err(StoreError::Inconsistent(
                    "record seq differs from last_signed_seq",
                ))
            } else {
                Ok(())
            }
        }
        None => Ok(()),
    }
}

/// Checks the rules every store enforces before writing a joined stick.
pub(crate) fn check_joined_stick(stick: &JoinedStick) -> Result<(), StoreError> {
    path_str(&stick.output_dir)?;
    check_time(stick.joined_at_ms)?;
    if let Some(completed) = &stick.completed {
        check_seq(completed.seq)?;
    }
    Ok(())
}

/// Checks that every number of a recorded file fits in an SQLite INTEGER.
pub(crate) fn check_owned_file(file: &OwnedFile) -> Result<(), StoreError> {
    check_stamp(&file.stamp)
}

pub(crate) fn check_stamp(stamp: &FileStamp) -> Result<(), StoreError> {
    for (name, value) in [
        ("file size", stamp.size),
        ("mtime", stamp.mtime_ns),
        ("ctime", stamp.ctime_ns),
    ] {
        if value > MAX_STORED_SEQ {
            return Err(StoreError::ValueOutOfRange(name, value));
        }
    }
    Ok(())
}

pub(crate) fn check_device(device: &Device) -> Result<(), StoreError> {
    if let SecretBackend::KeyFile(path) = &device.secret_backend {
        path_str(path)?;
    }
    check_time(device.created_at_ms)
}

#[cfg(test)]
mod tests;
