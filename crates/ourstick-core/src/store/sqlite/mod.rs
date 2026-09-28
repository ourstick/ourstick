//! [`Store`] on SQLite, with migrations from `crates/ourstick-core/migrations/`.

mod rows;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use rows::{blob32, to_u64, JoinedRow, OwnedRow, JOINED_COLUMNS, OWNED_COLUMNS};

use super::{
    check_device, check_joined_stick, check_owned_stick, check_seq, check_time, path_str, Device,
    JoinedStick, OwnedStick, SecretBackend, Store, StoreError,
};
use crate::record::{ManifestRecord, SignedManifestRecord};
use crate::stick_id::StickId;
use crate::ticket::NodeId;

/// Migration `i` (0-based) brings the schema from version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[include_str!("../../../migrations/0001_initial.sql")];

/// The schema version this code creates and understands.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

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

fn user_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Applies missing migrations, one transaction each. The version is read inside
/// the (immediate, i.e. write-locked) transaction, so two processes opening the
/// same database at once cannot apply a migration twice.
fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    loop {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version = user_version(&transaction)?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::NewerDatabase {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        let migration = MIGRATIONS[usize::try_from(version).expect("0 <= version < len")];
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", version + 1)?;
        transaction.commit()?;
        tracing::info!(version = version + 1, "database migrated");
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
                "INSERT INTO owned_sticks ({OWNED_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
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

    fn save_signed_record(&self, record: &SignedManifestRecord) -> Result<(), StoreError> {
        let content = record.unverified_record();
        let seq = seq_i64(content.seq)?;
        let stick_id = content.stick_id.to_bytes();
        let connection = self.connection();
        let updated = connection.execute(
            "UPDATE owned_sticks SET last_signed_seq = ?1, latest_record = ?2
             WHERE stick_id = ?3 AND last_signed_seq < ?1",
            params![seq, record.to_bytes().to_vec(), stick_id],
        )?;
        if updated == 1 {
            return Ok(());
        }
        let stored: Option<i64> = connection
            .query_row(
                "SELECT last_signed_seq FROM owned_sticks WHERE stick_id = ?1",
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

    fn add_joined_stick(&self, stick: &JoinedStick) -> Result<(), StoreError> {
        check_joined_stick(stick)?;
        let (seq, hash) = match &stick.current_manifest {
            Some(current) => (Some(seq_i64(current.seq)?), Some(current.hash.to_vec())),
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

    fn accept_record(&self, record: &ManifestRecord) -> Result<(), StoreError> {
        let seq = seq_i64(record.seq)?;
        let stick_id = record.stick_id.to_bytes();
        let connection = self.connection();
        let updated = connection.execute(
            "UPDATE joined_sticks SET highest_seq = ?1, current_manifest_hash = ?2
             WHERE stick_id = ?3 AND (highest_seq IS NULL OR highest_seq < ?1)",
            params![seq, record.manifest_hash.to_vec(), stick_id],
        )?;
        if updated == 1 {
            return Ok(());
        }
        let stored: Option<Option<i64>> = connection
            .query_row(
                "SELECT highest_seq FROM joined_sticks WHERE stick_id = ?1",
                [stick_id],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            None => Err(StoreError::StickNotFound),
            Some(stored) => Err(StoreError::NotNewer {
                seq: record.seq,
                stored: to_u64(stored.unwrap_or(0), "highest_seq")?,
            }),
        }
    }
}

#[cfg(test)]
mod tests;
