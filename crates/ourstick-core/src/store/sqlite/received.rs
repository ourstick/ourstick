//! A received (joined) stick's life in the receiver window (step 7b): its
//! name, program and pause, and forgetting it.

use rusqlite::{params, Connection, TransactionBehavior};

use super::super::{JoinedChange, JoinedStick, StoreError};
use super::rows::{kind_str, JoinedRow, JOINED_COLUMNS};
use crate::stick_id::StickId;

pub(super) fn change(
    connection: &Connection,
    stick_id: &StickId,
    change: &JoinedChange,
) -> Result<(), StoreError> {
    let id = stick_id.to_bytes();
    let live = "WHERE stick_id = ?2 AND forgetting = 0";
    let updated = match change {
        JoinedChange::Name(name) => connection.execute(
            &format!("UPDATE joined_sticks SET name = ?1 {live}"),
            params![name, id],
        )?,
        JoinedChange::Kind(kind) => connection.execute(
            &format!("UPDATE joined_sticks SET kind = ?1 {live}"),
            params![kind_str(*kind), id],
        )?,
        JoinedChange::Paused(paused) => connection.execute(
            &format!("UPDATE joined_sticks SET paused = ?1 {live}"),
            params![paused, id],
        )?,
        JoinedChange::Forgetting => connection.execute(
            &format!("UPDATE joined_sticks SET forgetting = ?1 {live}"),
            params![true, id],
        )?,
    };
    if updated == 1 {
        Ok(())
    } else {
        Err(StoreError::StickNotFound)
    }
}

pub(super) fn forgetting(connection: &Connection) -> Result<Vec<JoinedStick>, StoreError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {JOINED_COLUMNS} FROM joined_sticks WHERE forgetting = 1
         ORDER BY joined_at_ms, stick_id"
    ))?;
    let rows = statement.query_map([], JoinedRow::read)?;
    rows.map(|row| row?.into_stick()).collect()
}

pub(super) fn forget(connection: &mut Connection, stick_id: &StickId) -> Result<(), StoreError> {
    let id = stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let marked: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM joined_sticks WHERE stick_id = ?1 AND forgetting = 1)",
        [id],
        |row| row.get(0),
    )?;
    if !marked {
        return Err(StoreError::Inconsistent(
            "only a received stick marked as being forgotten is forgotten",
        ));
    }
    // Its record floor stays (ADR 0004): the same link added again must not
    // take an older version.
    for sql in [
        "DELETE FROM downloaded_files WHERE stick_id = ?1",
        "DELETE FROM space_claims WHERE stick_id = ?1 AND kind IN ('download', 'scratch')",
        "DELETE FROM joined_sticks WHERE stick_id = ?1",
    ] {
        transaction.execute(sql, [id])?;
    }
    transaction.commit()?;
    Ok(())
}
