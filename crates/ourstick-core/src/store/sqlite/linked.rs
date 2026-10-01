//! Linked entries: folders and files shared where they are (ADR 0008).

use rusqlite::{params, Connection, TransactionBehavior};

use super::super::{check_linked_entry, path_str, LinkedEntry, StoreError};
use super::rows::{is_constraint_violation, time_i64, to_u64};
use crate::stick_id::StickId;

pub(super) fn add(
    connection: &Connection,
    stick_id: &StickId,
    entry: &LinkedEntry,
) -> Result<(), StoreError> {
    check_linked_entry(entry)?;
    let known: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM owned_sticks WHERE stick_id = ?1 AND removing = 0)",
        [stick_id.to_bytes()],
        |row| row.get(0),
    )?;
    if !known {
        return Err(StoreError::StickNotFound);
    }
    let inserted = connection.execute(
        "INSERT INTO linked_entries (stick_id, name, path, is_dir, disk, added_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            stick_id.to_bytes(),
            entry.name,
            path_str(&entry.path)?,
            entry.is_dir,
            // A device id is an opaque 64-bit value: stored bit for bit.
            entry.disk.map(|disk| disk as i64),
            time_i64(entry.added_at_ms)?,
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
        "SELECT name, path, is_dir, disk, added_at_ms FROM linked_entries
         WHERE stick_id = ?1 ORDER BY name",
    )?;
    let rows = statement.query_map([stick_id.to_bytes()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (name, path, is_dir, disk, added_at_ms) = row?;
        Ok(LinkedEntry {
            name,
            path: path.into(),
            is_dir,
            disk: disk.map(|disk| disk as u64),
            added_at_ms: to_u64(added_at_ms, "added_at_ms")?,
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
    let removed = transaction.execute(
        "DELETE FROM linked_entries WHERE stick_id = ?1 AND name = ?2",
        params![stick_id.to_bytes(), name],
    )?;
    if removed != 1 {
        return Err(StoreError::LinkNotFound(name.to_owned()));
    }
    // Its files' rows: the path is the name, or starts with "name/".
    transaction.execute(
        "DELETE FROM owned_files WHERE stick_id = ?1
         AND (path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')",
        params![stick_id.to_bytes(), name],
    )?;
    transaction.commit()?;
    Ok(())
}
