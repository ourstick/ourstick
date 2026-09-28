//! Conversion between SQLite rows and store types, with checks on what is read.

use std::path::PathBuf;

use rusqlite::Row;

use super::super::{CurrentManifest, JoinedStick, OwnedStick, StoreError};
use crate::cipher::CipherId;
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;

pub(super) fn to_u64(value: i64, column: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("negative {column}")))
}

pub(super) fn blob32(bytes: Vec<u8>, column: &str) -> Result<[u8; 32], StoreError> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt(format!("{column} is not 32 bytes")))
}

fn stick_id_from(bytes: Vec<u8>) -> Result<StickId, StoreError> {
    StickId::from_bytes(&blob32(bytes, "stick_id")?)
        .map_err(|_| StoreError::Corrupt("invalid stick_id".to_owned()))
}

/// The raw columns of an `owned_sticks` row, converted after the query.
pub(super) struct OwnedRow {
    stick_id: Vec<u8>,
    name: String,
    root_path: String,
    cipher: i64,
    last_signed_seq: i64,
    latest_record: Option<Vec<u8>>,
    created_at_ms: i64,
}

pub(super) const OWNED_COLUMNS: &str =
    "stick_id, name, root_path, cipher, last_signed_seq, latest_record, created_at_ms";

impl OwnedRow {
    pub(super) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            stick_id: row.get(0)?,
            name: row.get(1)?,
            root_path: row.get(2)?,
            cipher: row.get(3)?,
            last_signed_seq: row.get(4)?,
            latest_record: row.get(5)?,
            created_at_ms: row.get(6)?,
        })
    }

    pub(super) fn into_stick(self) -> Result<OwnedStick, StoreError> {
        let cipher = u8::try_from(self.cipher)
            .ok()
            .and_then(|byte| CipherId::from_byte(byte).ok())
            .ok_or_else(|| StoreError::Corrupt(format!("unknown cipher {}", self.cipher)))?;
        let latest_record = self
            .latest_record
            .map(|bytes| {
                SignedManifestRecord::from_bytes(&bytes)
                    .map_err(|error| StoreError::Corrupt(format!("latest_record: {error}")))
            })
            .transpose()?;
        Ok(OwnedStick {
            stick_id: stick_id_from(self.stick_id)?,
            name: self.name,
            root_path: PathBuf::from(self.root_path),
            cipher,
            last_signed_seq: to_u64(self.last_signed_seq, "last_signed_seq")?,
            latest_record,
            created_at_ms: to_u64(self.created_at_ms, "created_at_ms")?,
        })
    }
}

pub(super) struct JoinedRow {
    stick_id: Vec<u8>,
    output_dir: String,
    highest_seq: Option<i64>,
    current_manifest_hash: Option<Vec<u8>>,
    joined_at_ms: i64,
}

pub(super) const JOINED_COLUMNS: &str =
    "stick_id, output_dir, highest_seq, current_manifest_hash, joined_at_ms";

impl JoinedRow {
    pub(super) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            stick_id: row.get(0)?,
            output_dir: row.get(1)?,
            highest_seq: row.get(2)?,
            current_manifest_hash: row.get(3)?,
            joined_at_ms: row.get(4)?,
        })
    }

    pub(super) fn into_stick(self) -> Result<JoinedStick, StoreError> {
        let current_manifest = match (self.highest_seq, self.current_manifest_hash) {
            (Some(seq), Some(hash)) => Some(CurrentManifest {
                seq: to_u64(seq, "highest_seq")?,
                hash: blob32(hash, "current_manifest_hash")?,
            }),
            (None, None) => None,
            _ => return Err(StoreError::Corrupt("half-set current manifest".to_owned())),
        };
        Ok(JoinedStick {
            stick_id: stick_id_from(self.stick_id)?,
            output_dir: PathBuf::from(self.output_dir),
            current_manifest,
            joined_at_ms: to_u64(self.joined_at_ms, "joined_at_ms")?,
        })
    }
}
