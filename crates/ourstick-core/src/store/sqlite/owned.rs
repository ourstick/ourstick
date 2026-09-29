//! Owned sticks: where a stick's folder is, and the records its owner signs.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{path_str, StoreError};
use super::rows::to_u64;
use super::seq_i64;
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;

pub(super) fn move_owned_stick(
    connection: &Connection,
    stick_id: &StickId,
    root: &Path,
) -> Result<(), StoreError> {
    let updated = connection.execute(
        "UPDATE owned_sticks SET root_path = ?1 WHERE stick_id = ?2",
        params![path_str(root)?, stick_id.to_bytes()],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(StoreError::StickNotFound)
    }
}

pub(super) fn save_signed_record(
    connection: &Connection,
    record: &SignedManifestRecord,
) -> Result<(), StoreError> {
    let content = record.unverified_record();
    let seq = seq_i64(content.seq)?;
    let stick_id = content.stick_id.to_bytes();
    let updated = connection.execute(
        "UPDATE owned_sticks SET last_signed_seq = ?1, latest_record = ?2
             WHERE stick_id = ?3 AND last_signed_seq < ?1",
        params![seq, record.to_bytes().to_vec(), stick_id],
    )?;
    if updated == 1 {
        return Ok(());
    }
    let stored: Option<i64> = connection
        .query_row(
            "SELECT last_signed_seq FROM owned_sticks WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )
        .optional()?;
    match stored {
        None => Err(StoreError::StickNotFound),
        Some(stored) => Err(StoreError::NotNewer {
            seq: content.seq,
            stored: to_u64(stored, "last_signed_seq")?,
        }),
    }
}
