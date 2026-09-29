//! Schema migrations: `crates/ourstick-core/migrations/NNNN_*.sql`, applied in
//! order, one transaction each.

use rusqlite::{Connection, TransactionBehavior};

use super::super::StoreError;

/// Migration `i` (0-based) brings the schema from version `i` to `i + 1`.
pub(super) const MIGRATIONS: &[&str] = &[
    include_str!("../../../migrations/0001_initial.sql"),
    include_str!("../../../migrations/0002_record_floors.sql"),
    include_str!("../../../migrations/0003_owned_files.sql"),
    include_str!("../../../migrations/0004_downloaded_files.sql"),
    include_str!("../../../migrations/0005_drive_size.sql"),
    include_str!("../../../migrations/0006_keychain_backend.sql"),
];

/// The schema version this code creates and understands.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub(super) fn user_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Applies missing migrations, one transaction each. The version is read inside
/// the (immediate, i.e. write-locked) transaction, so two processes opening the
/// same database at once cannot apply a migration twice.
pub(super) fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
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
