//! Conversion between SQLite rows and store types, with checks on what is read.

use std::path::PathBuf;

use rusqlite::Row;

use super::super::{
    check_seq, check_time, JoinedStick, ManifestVersion, OwnedStick, StickKind, StoreError,
};
use crate::cipher::CipherId;
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::NodeId;

pub(super) fn seq_i64(seq: u64) -> Result<i64, StoreError> {
    check_seq(seq)?;
    Ok(seq as i64)
}

pub(super) fn time_i64(ms: u64) -> Result<i64, StoreError> {
    check_time(ms)?;
    Ok(ms as i64)
}

pub(super) fn is_constraint_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

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
    drive_size: Option<i64>,
    drive_letter: Option<String>,
    latest_partial: bool,
    node_id: Option<Vec<u8>>,
    kind: String,
}

pub(super) const OWNED_COLUMNS: &str = "stick_id, name, root_path, cipher, last_signed_seq, \
     latest_record, created_at_ms, drive_size, drive_letter, latest_partial, node_id, kind";

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
            drive_size: row.get(7)?,
            drive_letter: row.get(8)?,
            latest_partial: row.get(9)?,
            node_id: row.get(10)?,
            kind: row.get(11)?,
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
            latest_partial: self.latest_partial,
            node_id: self
                .node_id
                .map(|node_id| blob32(node_id, "node_id").map(NodeId))
                .transpose()?,
            kind: match self.kind.as_str() {
                "app" => StickKind::App,
                "cli" => StickKind::Cli,
                other => return Err(StoreError::Corrupt(format!("kind {other}"))),
            },
            created_at_ms: to_u64(self.created_at_ms, "created_at_ms")?,
            drive_size: self
                .drive_size
                .map(|size| to_u64(size, "drive_size"))
                .transpose()?,
            drive_letter: self
                .drive_letter
                .map(|letter| {
                    let mut chars = letter.chars();
                    match (chars.next(), chars.next()) {
                        (Some(letter), None) if letter.is_ascii_uppercase() => Ok(letter),
                        _ => Err(StoreError::Corrupt(format!("drive_letter {letter}"))),
                    }
                })
                .transpose()?,
        })
    }
}

pub(super) struct JoinedRow {
    stick_id: Vec<u8>,
    output_dir: String,
    completed_seq: Option<i64>,
    completed_manifest_hash: Option<Vec<u8>>,
    joined_at_ms: i64,
}

pub(super) const JOINED_COLUMNS: &str =
    "stick_id, output_dir, completed_seq, completed_manifest_hash, joined_at_ms";

impl JoinedRow {
    pub(super) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            stick_id: row.get(0)?,
            output_dir: row.get(1)?,
            completed_seq: row.get(2)?,
            completed_manifest_hash: row.get(3)?,
            joined_at_ms: row.get(4)?,
        })
    }

    pub(super) fn into_stick(self) -> Result<JoinedStick, StoreError> {
        let completed = match (self.completed_seq, self.completed_manifest_hash) {
            (Some(seq), Some(hash)) => Some(ManifestVersion {
                seq: to_u64(seq, "completed_seq")?,
                hash: blob32(hash, "completed_manifest_hash")?,
            }),
            (None, None) => None,
            _ => {
                return Err(StoreError::Corrupt(
                    "half-set completed manifest".to_owned(),
                ))
            }
        };
        Ok(JoinedStick {
            stick_id: stick_id_from(self.stick_id)?,
            output_dir: PathBuf::from(self.output_dir),
            completed,
            joined_at_ms: to_u64(self.joined_at_ms, "joined_at_ms")?,
        })
    }
}

/// How a [`StickKind`] is stored.
pub(super) fn kind_str(kind: StickKind) -> &'static str {
    match kind {
        StickKind::App => "app",
        StickKind::Cli => "cli",
    }
}
