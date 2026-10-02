//! The space ledger's claims.

use rusqlite::{params, Connection, TransactionBehavior};

use super::super::{check_value, ClaimKind, SpaceClaim, StoreError};
use super::rows::{blob32, to_u64};
use crate::stick_id::StickId;

fn kind_str(kind: ClaimKind) -> &'static str {
    match kind {
        ClaimKind::Cache => "cache",
        ClaimKind::Download => "download",
        ClaimKind::Scratch => "scratch",
    }
}

fn read_all(connection: &Connection) -> Result<Vec<SpaceClaim>, StoreError> {
    let mut statement = connection
        .prepare("SELECT stick_id, kind, disk, bytes FROM space_claims ORDER BY stick_id, kind")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut claims = Vec::new();
    for row in rows {
        let (stick_id, kind, disk, bytes) = row?;
        let stick_id = StickId::from_bytes(&blob32(stick_id, "stick_id")?)
            .map_err(|_| StoreError::Corrupt("a claim's stick id is not a key".to_owned()))?;
        let kind = match kind.as_str() {
            "cache" => ClaimKind::Cache,
            "download" => ClaimKind::Download,
            "scratch" => ClaimKind::Scratch,
            // A kind of a later version of OurStick, running beside this
            // one: not counted, rather than every claim failing.
            other => {
                tracing::warn!(kind = other, "a claim of an unknown kind is not counted");
                continue;
            }
        };
        claims.push(SpaceClaim {
            stick_id,
            kind,
            disk,
            bytes: to_u64(bytes, "bytes")?,
        });
    }
    Ok(claims)
}

pub(super) fn claims(connection: &Connection) -> Result<Vec<SpaceClaim>, StoreError> {
    read_all(connection)
}

fn upsert(connection: &Connection, claim: &SpaceClaim) -> Result<(), StoreError> {
    check_value("bytes", claim.bytes)?;
    if claim.disk.is_empty() {
        return Err(StoreError::Inconsistent("a claim names its disk"));
    }
    connection.execute(
        "INSERT INTO space_claims (stick_id, kind, disk, bytes) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (stick_id, kind) DO UPDATE SET disk = ?3, bytes = ?4",
        params![
            claim.stick_id.to_bytes(),
            kind_str(claim.kind),
            claim.disk,
            claim.bytes as i64
        ],
    )?;
    Ok(())
}

pub(super) fn claim(
    connection: &mut Connection,
    decide: &mut dyn FnMut(&[SpaceClaim]) -> Vec<SpaceClaim>,
) -> Result<Vec<SpaceClaim>, StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let all = read_all(&transaction)?;
    let decided = decide(&all);
    for claim in &decided {
        upsert(&transaction, claim)?;
    }
    transaction.commit()?;
    Ok(decided)
}

pub(super) fn update(
    connection: &Connection,
    stick_id: &StickId,
    kind: ClaimKind,
    bytes: u64,
) -> Result<(), StoreError> {
    check_value("bytes", bytes)?;
    connection.execute(
        "UPDATE space_claims SET bytes = ?1 WHERE stick_id = ?2 AND kind = ?3",
        params![bytes as i64, stick_id.to_bytes(), kind_str(kind)],
    )?;
    Ok(())
}

pub(super) fn remove(
    connection: &Connection,
    stick_id: &StickId,
    kind: ClaimKind,
) -> Result<(), StoreError> {
    connection.execute(
        "DELETE FROM space_claims WHERE stick_id = ?1 AND kind = ?2",
        params![stick_id.to_bytes(), kind_str(kind)],
    )?;
    Ok(())
}
