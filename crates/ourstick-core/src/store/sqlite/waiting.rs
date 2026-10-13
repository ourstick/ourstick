//! Links the app waits to receive (ADR 0025): a stick waits here or is
//! received, never both.

use rusqlite::{params, Connection, TransactionBehavior};

use super::super::{StoreError, WaitingLink};
use super::is_constraint_violation;
use super::rows::{stick_id_from, time_i64, to_u64};
use crate::stick_id::StickId;

pub(super) fn add_waiting_link(
    connection: &mut Connection,
    link: &WaitingLink,
) -> Result<(), StoreError> {
    let stick_id = link.stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let received: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM joined_sticks WHERE stick_id = ?1)",
        [stick_id],
        |row| row.get(0),
    )?;
    if received {
        return Err(StoreError::StickExists);
    }
    let inserted = transaction.execute(
        "INSERT INTO waiting_links (stick_id, added_at_ms) VALUES (?1, ?2)",
        params![stick_id, time_i64(link.added_at_ms)?],
    );
    match inserted {
        Ok(_) => {}
        Err(error) if is_constraint_violation(&error) => return Err(StoreError::StickExists),
        Err(error) => return Err(error.into()),
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn waiting_links(connection: &Connection) -> Result<Vec<WaitingLink>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT stick_id, added_at_ms FROM waiting_links ORDER BY added_at_ms, stick_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.map(|row| {
        let (stick_id, added_at_ms) = row?;
        Ok(WaitingLink {
            stick_id: stick_id_from(stick_id)?,
            added_at_ms: to_u64(added_at_ms, "added_at_ms")?,
        })
    })
    .collect()
}

pub(super) fn remove_waiting_link(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<(), StoreError> {
    connection.execute(
        "DELETE FROM waiting_links WHERE stick_id = ?1",
        [stick_id.to_bytes()],
    )?;
    Ok(())
}
