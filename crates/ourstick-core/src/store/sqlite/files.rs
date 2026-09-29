//! Per-file tables: what the owner last encrypted (`owned_files`).

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::super::{check_owned_file, OwnedFile, StoreError};
use super::rows::{blob32, to_u64};
use crate::manifest::FileStamp;
use crate::stick_id::StickId;

pub(super) fn owned_files(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Vec<OwnedFile>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT path, size, mtime_ns, ctime_ns, blob_hash FROM owned_files
         WHERE stick_id = ?1 ORDER BY path",
    )?;
    let rows = statement.query_map([stick_id.to_bytes()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (path, size, mtime_ns, ctime_ns, hash) = row?;
        Ok(OwnedFile {
            path,
            stamp: FileStamp {
                size: to_u64(size, "size")?,
                mtime_ns: to_u64(mtime_ns, "mtime_ns")?,
                ctime_ns: to_u64(ctime_ns, "ctime_ns")?,
            },
            blob_hash: blob32(hash, "blob_hash")?,
        })
    })
    .collect()
}

pub(super) fn replace_owned_files(
    connection: &mut Connection,
    stick_id: &StickId,
    files: &[OwnedFile],
) -> Result<(), StoreError> {
    for file in files {
        check_owned_file(file)?;
    }
    let stick_id = stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let owned: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM owned_sticks WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )
        .optional()?;
    if owned.is_none() {
        return Err(StoreError::StickNotFound);
    }
    transaction.execute("DELETE FROM owned_files WHERE stick_id = ?1", [stick_id])?;
    {
        let mut insert = transaction.prepare(
            "INSERT INTO owned_files (stick_id, path, size, mtime_ns, ctime_ns, blob_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for file in files {
            // Checked above: every value fits in i64.
            insert.execute(params![
                stick_id,
                file.path,
                file.stamp.size as i64,
                file.stamp.mtime_ns as i64,
                file.stamp.ctime_ns as i64,
                file.blob_hash.to_vec(),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}
