//! Joined sticks: where a reader puts a stick, and which version it completed.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::super::{check_joined_stick, path_str, JoinedStick, ManifestVersion, StoreError};
use super::is_constraint_violation;
use super::rows::{seq_i64, time_i64, JoinedRow, JOINED_COLUMNS};
use crate::stick_id::StickId;

pub(super) fn add_joined_stick(
    connection: &Connection,
    stick: &JoinedStick,
) -> Result<(), StoreError> {
    check_joined_stick(stick)?;
    let (seq, hash) = match &stick.completed {
        Some(completed) => (Some(seq_i64(completed.seq)?), Some(completed.hash.to_vec())),
        None => (None, None),
    };
    let inserted = connection.execute(
        &format!("INSERT INTO joined_sticks ({JOINED_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            stick.stick_id.to_bytes(),
            path_str(&stick.output_dir)?,
            seq,
            hash,
            time_i64(stick.joined_at_ms)?,
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(error) if is_constraint_violation(&error) => Err(StoreError::StickExists),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn joined_stick(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Option<JoinedStick>, StoreError> {
    connection
        .query_row(
            &format!("SELECT {JOINED_COLUMNS} FROM joined_sticks WHERE stick_id = ?1"),
            [stick_id.to_bytes()],
            JoinedRow::read,
        )
        .optional()?
        .map(JoinedRow::into_stick)
        .transpose()
}

pub(super) fn joined_sticks(connection: &Connection) -> Result<Vec<JoinedStick>, StoreError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {JOINED_COLUMNS} FROM joined_sticks ORDER BY joined_at_ms, stick_id"
    ))?;
    let rows = statement.query_map([], JoinedRow::read)?;
    rows.map(|row| row?.into_stick()).collect()
}

pub(super) fn move_joined_stick(
    connection: &mut Connection,
    stick_id: &StickId,
    output_dir: &Path,
) -> Result<(), StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE joined_sticks
         SET output_dir = ?1, completed_seq = NULL, completed_manifest_hash = NULL
         WHERE stick_id = ?2",
        params![path_str(output_dir)?, stick_id.to_bytes()],
    )?;
    if updated != 1 {
        return Err(StoreError::StickNotFound);
    }
    transaction.execute(
        "DELETE FROM downloaded_files WHERE stick_id = ?1",
        [stick_id.to_bytes()],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn complete_download(
    connection: &mut Connection,
    stick_id: &StickId,
    version: ManifestVersion,
) -> Result<(), StoreError> {
    let seq = seq_i64(version.seq)?;
    let stick_id = stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let floor: Option<i64> = transaction
        .query_row(
            "SELECT highest_seq FROM record_floors WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )
        .optional()?;
    if floor.is_none_or(|floor| floor < seq) {
        return Err(StoreError::Inconsistent(
            "a download completed for a record that was never accepted",
        ));
    }
    let updated = transaction.execute(
        "UPDATE joined_sticks SET completed_seq = ?1, completed_manifest_hash = ?2
         WHERE stick_id = ?3",
        params![seq, version.hash.to_vec(), stick_id],
    )?;
    if updated != 1 {
        return Err(StoreError::StickNotFound);
    }
    transaction.commit()?;
    Ok(())
}
