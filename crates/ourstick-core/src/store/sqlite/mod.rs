//! [`Store`] on SQLite, with migrations from `crates/ourstick-core/migrations/`.

mod files;
mod migrations;
mod owned;
mod rows;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[cfg(test)]
use migrations::MIGRATIONS;
pub use migrations::SCHEMA_VERSION;
use migrations::{migrate, user_version};
use rows::{blob32, to_u64, JoinedRow, OwnedRow, JOINED_COLUMNS, OWNED_COLUMNS};

use super::{
    check_device, check_joined_stick, check_owned_stick, check_seq, check_time, path_str, Device,
    DownloadedFile, JoinedStick, ManifestVersion, OwnedFile, OwnedStick, SecretBackend, Store,
    StoreError,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;
use crate::ticket::NodeId;

/// How long to wait for another process holding the database lock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A [`Store`] in a SQLite database.
#[derive(Debug)]
pub struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    /// Opens (or creates) the database at `path` and brings its schema up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            tracing::warn!(journal_mode, "database is not in WAL mode");
        }
        Self::setup(connection)
    }

    /// A fresh database in memory, for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(mut connection: Connection) -> Result<Self, StoreError> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// The schema version of the open database.
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(user_version(&self.connection())?)
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        // Every operation is a single statement or transaction, so a panic
        // while holding the lock leaves nothing half-done.
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn seq_i64(seq: u64) -> Result<i64, StoreError> {
    check_seq(seq)?;
    Ok(seq as i64)
}

fn time_i64(ms: u64) -> Result<i64, StoreError> {
    check_time(ms)?;
    Ok(ms as i64)
}

fn is_constraint_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

impl Store for SqliteStore {
    fn device(&self) -> Result<Option<Device>, StoreError> {
        let row = self
            .connection()
            .query_row(
                "SELECT node_id, secret_backend, key_file_path, created_at_ms FROM device",
                [],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((node_id, backend, key_file_path, created_at_ms)) = row else {
            return Ok(None);
        };
        let secret_backend = match (backend.as_str(), key_file_path) {
            ("secret-service", None) => SecretBackend::SecretService,
            ("keychain", None) => SecretBackend::Keychain,
            ("credential-manager", None) => SecretBackend::CredentialManager,
            ("key-file", Some(path)) => SecretBackend::KeyFile(PathBuf::from(path)),
            _ => return Err(StoreError::Corrupt(format!("secret_backend {backend}"))),
        };
        Ok(Some(Device {
            node_id: NodeId(blob32(node_id, "node_id")?),
            secret_backend,
            created_at_ms: to_u64(created_at_ms, "created_at_ms")?,
        }))
    }

    fn create_device(&self, device: &Device) -> Result<(), StoreError> {
        check_device(device)?;
        let (backend, key_file_path) = match &device.secret_backend {
            SecretBackend::SecretService => ("secret-service", None),
            SecretBackend::Keychain => ("keychain", None),
            SecretBackend::CredentialManager => ("credential-manager", None),
            SecretBackend::KeyFile(path) => ("key-file", Some(path_str(path)?)),
        };
        let inserted = self.connection().execute(
            "INSERT INTO device (id, node_id, secret_backend, key_file_path, created_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                device.node_id.0,
                backend,
                key_file_path,
                time_i64(device.created_at_ms)?
            ],
        );
        match inserted {
            Ok(_) => Ok(()),
            Err(error) if is_constraint_violation(&error) => Err(StoreError::DeviceExists),
            Err(error) => Err(error.into()),
        }
    }

    fn add_owned_stick(&self, stick: &OwnedStick) -> Result<(), StoreError> {
        check_owned_stick(stick)?;
        let inserted = self.connection().execute(
            &format!(
                "INSERT INTO owned_sticks ({OWNED_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
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
                // Checked by check_owned_stick: fits in i64.
                stick.drive_size.map(|size| size as i64),
                stick.drive_letter.map(String::from),
                stick.latest_partial,
            ],
        );
        match inserted {
            Ok(_) => Ok(()),
            Err(error) if is_constraint_violation(&error) => Err(StoreError::StickExists),
            Err(error) => Err(error.into()),
        }
    }

    fn owned_stick(&self, stick_id: &StickId) -> Result<Option<OwnedStick>, StoreError> {
        self.connection()
            .query_row(
                &format!("SELECT {OWNED_COLUMNS} FROM owned_sticks WHERE stick_id = ?1"),
                [stick_id.to_bytes()],
                OwnedRow::read,
            )
            .optional()?
            .map(OwnedRow::into_stick)
            .transpose()
    }

    fn owned_sticks(&self) -> Result<Vec<OwnedStick>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {OWNED_COLUMNS} FROM owned_sticks ORDER BY created_at_ms, stick_id"
        ))?;
        let rows = statement.query_map([], OwnedRow::read)?;
        rows.map(|row| row?.into_stick()).collect()
    }

    fn move_owned_stick(&self, stick_id: &StickId, root: &Path) -> Result<(), StoreError> {
        owned::move_owned_stick(&self.connection(), stick_id, root)
    }

    fn set_drive_letter(&self, stick_id: &StickId, letter: Option<char>) -> Result<(), StoreError> {
        owned::set_drive_letter(&self.connection(), stick_id, letter)
    }

    fn save_signed_record(
        &self,
        record: &SignedManifestRecord,
        partial: bool,
    ) -> Result<(), StoreError> {
        owned::save_signed_record(&self.connection(), record, partial)
    }

    fn owned_files(&self, stick_id: &StickId) -> Result<Vec<OwnedFile>, StoreError> {
        files::owned_files(&self.connection(), stick_id)
    }

    fn record_owned_files(
        &self,
        stick_id: &StickId,
        files: &[OwnedFile],
    ) -> Result<(), StoreError> {
        files::record_owned_files(&mut self.connection(), stick_id, files)
    }

    fn keep_only_owned_files(
        &self,
        stick_id: &StickId,
        paths: &[String],
    ) -> Result<(), StoreError> {
        files::keep_only_owned_files(&mut self.connection(), stick_id, paths)
    }

    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError> {
        check_joined_stick(stick)?;
        let (seq, hash) = match &stick.completed {
            Some(completed) => (Some(seq_i64(completed.seq)?), Some(completed.hash.to_vec())),
            None => (None, None),
        };
        let inserted = self.connection().execute(
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

    fn joined_stick(&self, stick_id: &StickId) -> Result<Option<JoinedStick>, StoreError> {
        self.connection()
            .query_row(
                &format!("SELECT {JOINED_COLUMNS} FROM joined_sticks WHERE stick_id = ?1"),
                [stick_id.to_bytes()],
                JoinedRow::read,
            )
            .optional()?
            .map(JoinedRow::into_stick)
            .transpose()
    }

    fn joined_sticks(&self) -> Result<Vec<JoinedStick>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {JOINED_COLUMNS} FROM joined_sticks ORDER BY joined_at_ms, stick_id"
        ))?;
        let rows = statement.query_map([], JoinedRow::read)?;
        rows.map(|row| row?.into_stick()).collect()
    }

    fn move_joined_stick(&self, stick_id: &StickId, output_dir: &Path) -> Result<(), StoreError> {
        let mut connection = self.connection();
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

    fn complete_download(
        &self,
        stick_id: &StickId,
        version: ManifestVersion,
    ) -> Result<(), StoreError> {
        let seq = seq_i64(version.seq)?;
        let stick_id = stick_id.to_bytes();
        let mut connection = self.connection();
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

    fn downloaded_files(&self, stick_id: &StickId) -> Result<Vec<DownloadedFile>, StoreError> {
        files::downloaded_files(&self.connection(), stick_id)
    }

    fn record_downloaded_file(
        &self,
        stick_id: &StickId,
        file: &DownloadedFile,
    ) -> Result<(), StoreError> {
        files::record_downloaded_file(&self.connection(), stick_id, file)
    }

    fn record_floor(&self, stick_id: &StickId) -> Result<Option<ManifestVersion>, StoreError> {
        let row: Option<(i64, Vec<u8>)> = self
            .connection()
            .query_row(
                "SELECT highest_seq, manifest_hash FROM record_floors WHERE stick_id = ?1",
                [stick_id.to_bytes()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(seq, hash)| {
            Ok(ManifestVersion {
                seq: to_u64(seq, "highest_seq")?,
                hash: blob32(hash, "manifest_hash")?,
            })
        })
        .transpose()
    }

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        let seq = seq_i64(record.seq)?;
        let stick_id = record.stick_id.to_bytes();
        let connection = self.connection();
        // One statement: insert, or raise an existing floor only if higher.
        let changed = connection.execute(
            "INSERT INTO record_floors (stick_id, highest_seq, manifest_hash)
             VALUES (?3, ?1, ?2)
             ON CONFLICT (stick_id) DO UPDATE
             SET highest_seq = excluded.highest_seq, manifest_hash = excluded.manifest_hash
             WHERE record_floors.highest_seq < excluded.highest_seq",
            params![seq, record.manifest_hash.to_vec(), stick_id],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let stored: i64 = connection.query_row(
            "SELECT highest_seq FROM record_floors WHERE stick_id = ?1",
            [stick_id],
            |row| row.get(0),
        )?;
        Err(StoreError::NotNewer {
            seq: record.seq,
            stored: to_u64(stored, "highest_seq")?,
        })
    }
}

#[cfg(test)]
mod tests;
