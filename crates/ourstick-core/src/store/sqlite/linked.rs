//! Linked entries: folders and files shared where they are (ADR 0008).

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::super::{
    check_at_top, check_linked_entry, path_str, LinkedEntry, StoreError, AT_TOP_STAYS,
};
use super::rows::{is_constraint_violation, time_i64, to_u64};
use crate::stick_id::StickId;

pub(super) fn add(
    connection: &Connection,
    stick_id: &StickId,
    entry: &LinkedEntry,
) -> Result<(), StoreError> {
    check_linked_entry(entry)?;
    let has_own_folder: Option<bool> = connection
        .query_row(
            "SELECT has_own_folder FROM owned_sticks WHERE stick_id = ?1 AND removing = 0",
            [stick_id.to_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(has_own_folder) = has_own_folder else {
        return Err(StoreError::StickNotFound);
    };
    if entry.at_top {
        let one_there: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM linked_entries WHERE stick_id = ?1 AND at_top = 1)",
            [stick_id.to_bytes()],
            |row| row.get(0),
        )?;
        check_at_top(has_own_folder, one_there)?;
    }
    let inserted = connection.execute(
        "INSERT INTO linked_entries (stick_id, name, path, is_dir, disk, added_at_ms, at_top)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            stick_id.to_bytes(),
            entry.name,
            path_str(&entry.path)?,
            entry.is_dir,
            // A device id is an opaque 64-bit value: stored bit for bit.
            entry.disk.map(|disk| disk as i64),
            time_i64(entry.added_at_ms)?,
            entry.at_top,
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(error) if is_constraint_violation(&error) => {
            Err(StoreError::LinkExists(entry.name.clone()))
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn entries(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Vec<LinkedEntry>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT name, path, is_dir, disk, added_at_ms, at_top FROM linked_entries
         WHERE stick_id = ?1 ORDER BY name",
    )?;
    let rows = statement.query_map([stick_id.to_bytes()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, bool>(5)?,
        ))
    })?;
    rows.map(|row| {
        let (name, path, is_dir, disk, added_at_ms, at_top) = row?;
        Ok(LinkedEntry {
            name,
            path: path.into(),
            is_dir,
            disk: disk.map(|disk| disk as u64),
            added_at_ms: to_u64(added_at_ms, "added_at_ms")?,
            at_top,
        })
    })
    .collect()
}

pub(super) fn remove(
    connection: &mut Connection,
    stick_id: &StickId,
    name: &str,
) -> Result<(), StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let at_top: Option<bool> = transaction
        .query_row(
            "SELECT at_top FROM linked_entries WHERE stick_id = ?1 AND name = ?2",
            params![stick_id.to_bytes(), name],
            |row| row.get(0),
        )
        .optional()?;
    match at_top {
        None => return Err(StoreError::LinkNotFound(name.to_owned())),
        Some(true) => return Err(StoreError::Inconsistent(AT_TOP_STAYS)),
        Some(false) => {}
    }
    transaction.execute(
        "DELETE FROM linked_entries WHERE stick_id = ?1 AND name = ?2",
        params![stick_id.to_bytes(), name],
    )?;
    // Its files' rows: the path is the name, or starts with "name/".
    transaction.execute(
        "DELETE FROM owned_files WHERE stick_id = ?1
         AND (path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')",
        params![stick_id.to_bytes(), name],
    )?;
    transaction.commit()?;
    Ok(())
}
