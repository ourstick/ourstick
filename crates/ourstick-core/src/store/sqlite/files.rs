//! Per-file tables: what the owner last encrypted (`owned_files`) and what
//! `get` wrote into a folder (`downloaded_files`).

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::super::{check_owned_file, check_stamp, DownloadedFile, OwnedFile, StoreError};
use super::rows::{blob32, to_u64};
use crate::manifest::FileStamp;
use crate::stick_id::StickId;

pub(super) fn owned_files(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Vec<OwnedFile>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT path, size, mtime_ns, ctime_ns, blob_hash, plaintext_hash FROM owned_files
         WHERE stick_id = ?1 ORDER BY path",
    )?;
    let rows = statement.query_map([stick_id.to_bytes()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, Option<Vec<u8>>>(5)?,
        ))
    })?;
    rows.map(|row| {
        let (path, size, mtime_ns, ctime_ns, hash, plaintext_hash) = row?;
        Ok(OwnedFile {
            path,
            stamp: FileStamp {
                size: to_u64(size, "size")?,
                mtime_ns: to_u64(mtime_ns, "mtime_ns")?,
                ctime_ns: to_u64(ctime_ns, "ctime_ns")?,
            },
            blob_hash: blob32(hash, "blob_hash")?,
            plaintext_hash: plaintext_hash
                .map(|hash| blob32(hash, "plaintext_hash"))
                .transpose()?,
        })
    })
    .collect()
}

/// Fails with `StickNotFound` unless `stick_id` is an owned stick.
fn check_owned(connection: &Connection, stick_id: &[u8; 32]) -> Result<(), StoreError> {
    let owned: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM owned_sticks WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )
        .optional()?;
    owned.map(|_| ()).ok_or(StoreError::StickNotFound)
}

pub(super) fn record_owned_files(
    connection: &mut Connection,
    stick_id: &StickId,
    files: &[OwnedFile],
) -> Result<(), StoreError> {
    for file in files {
        check_owned_file(file)?;
    }
    let stick_id = stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_owned(&transaction, &stick_id)?;
    {
        let mut insert = transaction.prepare(
            "INSERT INTO owned_files
                 (stick_id, path, size, mtime_ns, ctime_ns, blob_hash, plaintext_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (stick_id, path) DO UPDATE SET size = excluded.size,
                 mtime_ns = excluded.mtime_ns, ctime_ns = excluded.ctime_ns,
                 blob_hash = excluded.blob_hash, plaintext_hash = excluded.plaintext_hash",
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
                file.plaintext_hash.map(|hash| hash.to_vec()),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn keep_only_owned_files(
    connection: &mut Connection,
    stick_id: &StickId,
    paths: &[String],
) -> Result<(), StoreError> {
    let stick_id = stick_id.to_bytes();
    let keep: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_owned(&transaction, &stick_id)?;
    let recorded: Vec<String> = transaction
        .prepare("SELECT path FROM owned_files WHERE stick_id = ?1")?
        .query_map([stick_id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    {
        let mut delete =
            transaction.prepare("DELETE FROM owned_files WHERE stick_id = ?1 AND path = ?2")?;
        for path in recorded.iter().filter(|path| !keep.contains(path.as_str())) {
            delete.execute(params![stick_id, path])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn downloaded_files(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<Vec<DownloadedFile>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT path, size, mtime_ns, ctime_ns, blob_hash FROM downloaded_files
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
        Ok(DownloadedFile {
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

pub(super) fn record_downloaded_file(
    connection: &Connection,
    stick_id: &StickId,
    file: &DownloadedFile,
) -> Result<(), StoreError> {
    check_stamp(&file.stamp)?;
    let joined: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM joined_sticks WHERE stick_id = ?1",
            [stick_id.to_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    if joined.is_none() {
        return Err(StoreError::StickNotFound);
    }
    // Checked above: every value fits in i64.
    connection.execute(
        "INSERT INTO downloaded_files (stick_id, path, size, mtime_ns, ctime_ns, blob_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (stick_id, path) DO UPDATE SET size = excluded.size,
             mtime_ns = excluded.mtime_ns, ctime_ns = excluded.ctime_ns,
             blob_hash = excluded.blob_hash",
        params![
            stick_id.to_bytes(),
            file.path,
            file.stamp.size as i64,
            file.stamp.mtime_ns as i64,
            file.stamp.ctime_ns as i64,
            file.blob_hash.to_vec(),
        ],
    )?;
    Ok(())
}
