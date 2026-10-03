//! Opening the database: its pragmas, its migrations, its lock.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;

use super::migrations::{migrate, user_version};
use super::SqliteStore;
use crate::store::StoreError;

/// How long to wait for another process holding the database lock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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

    pub(super) fn connection(&self) -> MutexGuard<'_, Connection> {
        // Every operation is a single statement or transaction, so a panic
        // while holding the lock leaves nothing half-done.
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
