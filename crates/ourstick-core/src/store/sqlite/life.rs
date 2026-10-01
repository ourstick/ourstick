//! A stick's life in the app (ADR 0009 §5): rename, stop wanting it shared,
//! a new link (replace), removal, retired ids.

use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};

use super::super::{check_owned_stick, path_str, OwnedStick, StoreError};
use super::rows::{blob32, is_constraint_violation, time_i64, OwnedRow, OWNED_COLUMNS};
use crate::stick_id::StickId;

/// Runs `sql` on the live (not removing) stick `stick_id`.
fn update_live(
    connection: &Connection,
    sql: &str,
    value: &dyn rusqlite::ToSql,
    stick_id: &StickId,
) -> Result<(), StoreError> {
    let updated = connection.execute(sql, params![value, stick_id.to_bytes()])?;
    if updated == 1 {
        Ok(())
    } else {
        Err(StoreError::StickNotFound)
    }
}

pub(super) fn rename(
    connection: &Connection,
    stick_id: &StickId,
    name: &str,
) -> Result<(), StoreError> {
    update_live(
        connection,
        "UPDATE owned_sticks SET name = ?1 WHERE stick_id = ?2 AND removing = 0",
        &name,
        stick_id,
    )
}

pub(super) fn set_sharing_wanted(
    connection: &Connection,
    stick_id: &StickId,
    wanted: bool,
) -> Result<(), StoreError> {
    update_live(
        connection,
        "UPDATE owned_sticks SET sharing_wanted = ?1 WHERE stick_id = ?2 AND removing = 0",
        &wanted,
        stick_id,
    )
}

pub(super) fn set_image_path(
    connection: &Connection,
    stick_id: &StickId,
    path: &Path,
) -> Result<(), StoreError> {
    update_live(
        connection,
        "UPDATE owned_sticks SET image_path = ?1 WHERE stick_id = ?2 AND removing = 0",
        &path_str(path)?,
        stick_id,
    )
}

pub(super) fn removing(connection: &Connection) -> Result<Vec<OwnedStick>, StoreError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {OWNED_COLUMNS} FROM owned_sticks WHERE removing = 1
         ORDER BY created_at_ms, stick_id"
    ))?;
    let rows = statement.query_map([], OwnedRow::read)?;
    rows.map(|row| row?.into_stick()).collect()
}

pub(super) fn mark_removing(connection: &Connection, stick_id: &StickId) -> Result<(), StoreError> {
    update_live(
        connection,
        "UPDATE owned_sticks SET removing = ?1 WHERE stick_id = ?2 AND removing = 0",
        &true,
        stick_id,
    )
}

/// Deletes the rows that belong to `stick_id` in other tables, then its own.
fn delete_stick_rows(connection: &Connection, stick_id: &StickId) -> Result<(), StoreError> {
    for table in ["owned_files", "linked_entries", "space_claims"] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE stick_id = ?1"),
            [stick_id.to_bytes()],
        )?;
    }
    connection.execute(
        "DELETE FROM owned_sticks WHERE stick_id = ?1",
        [stick_id.to_bytes()],
    )?;
    Ok(())
}

pub(super) fn remove(connection: &mut Connection, stick_id: &StickId) -> Result<(), StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let marked: Option<bool> = transaction
        .query_row(
            "SELECT removing FROM owned_sticks WHERE stick_id = ?1",
            [stick_id.to_bytes()],
            |row| row.get(0),
        )
        .ok();
    match marked {
        None => return Err(StoreError::StickNotFound),
        Some(false) => {
            return Err(StoreError::Inconsistent(
                "only a stick marked as being removed is deleted",
            ))
        }
        Some(true) => {}
    }
    delete_stick_rows(&transaction, stick_id)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn replace(
    connection: &mut Connection,
    old: &StickId,
    new: &OwnedStick,
    now_ms: u64,
) -> Result<(), StoreError> {
    check_owned_stick(new)?;
    if new.stick_id == *old {
        return Err(StoreError::Inconsistent("a new link is a new stick id"));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let live: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM owned_sticks WHERE stick_id = ?1 AND removing = 0)",
        [old.to_bytes()],
        |row| row.get(0),
    )?;
    if !live {
        return Err(StoreError::StickNotFound);
    }
    super::owned::insert_owned(&transaction, new).map_err(|error| match error {
        StoreError::Sqlite(error) if is_constraint_violation(&error) => StoreError::StickExists,
        error => error,
    })?;
    transaction.execute(
        "UPDATE linked_entries SET stick_id = ?1 WHERE stick_id = ?2",
        params![new.stick_id.to_bytes(), old.to_bytes()],
    )?;
    delete_stick_rows(&transaction, old)?;
    transaction.execute(
        "INSERT INTO retired_sticks (stick_id, retired_at_ms) VALUES (?1, ?2)",
        params![old.to_bytes(), time_i64(now_ms)?],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn retired(connection: &Connection) -> Result<Vec<StickId>, StoreError> {
    let mut statement = connection
        .prepare("SELECT stick_id FROM retired_sticks ORDER BY retired_at_ms, stick_id")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    rows.map(|bytes| {
        StickId::from_bytes(&blob32(bytes?, "stick_id")?)
            .map_err(|_| StoreError::Corrupt("a retired id is not a key".to_owned()))
    })
    .collect()
}

pub(super) fn forget_retired(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<(), StoreError> {
    connection.execute(
        "DELETE FROM retired_sticks WHERE stick_id = ?1",
        [stick_id.to_bytes()],
    )?;
    Ok(())
}
