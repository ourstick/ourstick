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
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;
use crate::ticket::NodeId;

pub use memory::MemoryStore;
pub use sqlite::{SqliteStore, SCHEMA_VERSION};

/// Where this device keeps its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretBackend {
    SecretService,
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
    pub created_at_ms: u64,
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
            created_at_ms,
        }
    }
}

/// The manifest a reader currently holds: from the last accepted record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentManifest {
    pub seq: u64,
    pub hash: [u8; 32],
}

/// A stick this device reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedStick {
    pub stick_id: StickId,
    pub output_dir: PathBuf,
    /// From the last accepted signed record; `None` before the first one.
    /// Its `seq` is the `last_seen` of ADR 0004.
    pub current_manifest: Option<CurrentManifest>,
    pub joined_at_ms: u64,
}

impl JoinedStick {
    /// A stick joined from a ticket, before any record was accepted.
    pub fn new(stick_id: StickId, output_dir: impl Into<PathBuf>, joined_at_ms: u64) -> Self {
        Self {
            stick_id,
            output_dir: output_dir.into(),
            current_manifest: None,
            joined_at_ms,
        }
    }
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

    /// Stores a record the owner just signed. Fails with
    /// [`StoreError::NotNewer`] unless its `seq` is greater than the stored one,
    /// checked and updated atomically.
    fn save_signed_record(&self, record: &SignedManifestRecord) -> Result<(), StoreError>;

    /// Records a new joined stick. Fails if it exists.
    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError>;

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError>;

    /// All joined sticks, oldest first.
    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError>;

    /// Stores a record the reader has verified. Fails with
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
pub(crate) fn check_owned_stick(stick: &OwnedStick) -> Result<(), StoreError> {
    check_seq(stick.last_signed_seq)?;
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
    if let Some(current) = &stick.current_manifest {
        check_seq(current.seq)?;
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
