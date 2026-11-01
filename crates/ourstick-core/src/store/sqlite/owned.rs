//! Owned sticks: where a stick's folder is, and the records its owner signs.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{check_drive_letter, path_str, OwnedStick, StoreError};
use super::rows::{kind_str, seq_i64, time_i64, to_u64, OwnedRow, OWNED_COLUMNS};
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;

/// The owned stick `stick_id`, unless it is being removed.
pub(super) fn stick(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Option<OwnedStick>, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {OWNED_COLUMNS} FROM owned_sticks WHERE stick_id = ?1 AND removing = 0"
            ),
            [stick_id.to_bytes()],
            OwnedRow::read,
        )
        .optional()?
        .map(OwnedRow::into_stick)
        .transpose()
}

pub(super) fn move_owned_stick(
    connection: &Connection,
    stick_id: &StickId,
    root: &Path,
) -> Result<(), StoreError> {
    let updated = connection.execute(
        "UPDATE owned_sticks SET root_path = ?1 WHERE stick_id = ?2 AND removing = 0",
        params![path_str(root)?, stick_id.to_bytes()],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(StoreError::StickNotFound)
    }
}

pub(super) fn set_drive_letter(
    connection: &Connection,
    stick_id: &StickId,
    letter: Option<char>,
) -> Result<(), StoreError> {
    check_drive_letter(letter)?;
    let updated = connection.execute(
        "UPDATE owned_sticks SET drive_letter = ?1 WHERE stick_id = ?2 AND removing = 0",
        params![letter.map(String::from), stick_id.to_bytes()],
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
    partial: bool,
) -> Result<(), StoreError> {
    let content = record.unverified_record();
    let seq = seq_i64(content.seq)?;
    let stick_id = content.stick_id.to_bytes();
    let updated = connection.execute(
        "UPDATE owned_sticks SET last_signed_seq = ?1, latest_record = ?2, latest_partial = ?4
             WHERE stick_id = ?3 AND last_signed_seq < ?1 AND removing = 0",
        params![seq, record.to_bytes().to_vec(), stick_id, partial],
    )?;
    if updated == 1 {
        return Ok(());
    }
    let stored: Option<i64> = connection
        .query_row(
            "SELECT last_signed_seq FROM owned_sticks WHERE stick_id = ?1 AND removing = 0",
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

/// Inserts an owned stick's row (checked by the caller).
pub(super) fn insert_owned(connection: &Connection, stick: &OwnedStick) -> Result<(), StoreError> {
    connection.execute(
        &format!(
            "INSERT INTO owned_sticks ({OWNED_COLUMNS}) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
        ),
        params![
            stick.stick_id.to_bytes(),
            stick.name,
            path_str(&stick.root_path)?,
            stick.cipher.to_byte(),
            seq_i64(stick.last_signed_seq)?,
            stick
                .latest_record
                .as_ref()
                .map(|record| record.to_bytes().to_vec()),
            time_i64(stick.created_at_ms)?,
            stick.drive_letter.map(String::from),
            stick.latest_partial,
            stick.node_id.map(|node_id| node_id.0.to_vec()),
            kind_str(stick.kind),
            stick.image_path.as_deref().map(path_str).transpose()?,
            stick.sharing_wanted,
            stick.has_own_folder,
        ],
    )?;
    Ok(())
}
