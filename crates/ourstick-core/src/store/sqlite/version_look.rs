//! `version_look` and `notice_serials` (ADR 0033): what the app keeps of its
//! looks for a new version.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::super::StoreError;
use super::rows::{time_i64, to_u64};
use super::SqliteStore;
use crate::new_version::{LookFailure, NoticeStore, VersionLook};

const COLUMNS: &str = "looking, looking_told, next_look_ms, last_good_ms, failing_since_ms,
    failure, newest, security, new_release_key, security_told";

type Row = (
    bool,
    bool,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl NoticeStore for SqliteStore {
    fn version_look(&self) -> Result<VersionLook, StoreError> {
        let connection = self.connection();
        let row: Option<Row> = connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM version_look WHERE id = 1"),
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()?;
        let mut look = VersionLook::default();
        if let Some((looking, told, next, good, failing, failure, newest, security, key, told_of)) =
            row
        {
            let time = |ms: Option<i64>, column| ms.map(|ms| to_u64(ms, column)).transpose();
            look = VersionLook {
                looking,
                looking_told: told,
                next_look_ms: to_u64(next, "next_look_ms")?,
                last_good_ms: time(good, "last_good_ms")?,
                failing_since_ms: time(failing, "failing_since_ms")?,
                failure: failure
                    .map(|text| {
                        LookFailure::parse(&text)
                            .ok_or_else(|| StoreError::Corrupt(format!("failure {text}")))
                    })
                    .transpose()?,
                newest,
                security,
                new_release_key: key,
                security_told: told_of,
                ..VersionLook::default()
            };
        }
        let mut statement = connection.prepare("SELECT key_id, serial FROM notice_serials")?;
        for serial in statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))? {
            let (key_id, serial): (String, i64) = serial?;
            look.serials.insert(key_id, to_u64(serial, "serial")?);
        }
        Ok(look)
    }

    fn set_version_look(&self, look: &VersionLook) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let time = |ms: Option<u64>| ms.map(time_i64).transpose();
        transaction.execute(
            &format!(
                "INSERT OR REPLACE INTO version_look (id, {COLUMNS})
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                look.looking,
                look.looking_told,
                time_i64(look.next_look_ms)?,
                time(look.last_good_ms)?,
                time(look.failing_since_ms)?,
                look.failure.map(LookFailure::as_str),
                look.newest,
                look.security,
                look.new_release_key,
                look.security_told,
            ],
        )?;
        transaction.execute("DELETE FROM notice_serials", [])?;
        for (key_id, serial) in &look.serials {
            let serial = i64::try_from(*serial)
                .map_err(|_| StoreError::Corrupt("a serial too large".into()))?;
            transaction.execute(
                "INSERT INTO notice_serials (key_id, serial) VALUES (?1, ?2)",
                params![key_id, serial],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn nothing_kept_means_the_defaults() {
        let store = store();
        assert_eq!(
            store.version_look().unwrap(),
            VersionLook {
                // A fresh database has no device: nothing to tell.
                looking_told: true,
                ..VersionLook::default()
            }
        );
    }

    /// A device set up before this version is told once that the app now
    /// looks (rule 11).
    #[test]
    fn an_upgraded_device_is_told_once() {
        use super::super::migrations::MIGRATIONS;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.db");
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            for migration in &MIGRATIONS[..28] {
                connection.execute_batch(migration).unwrap();
            }
            connection.pragma_update(None, "user_version", 28).unwrap();
            connection
                .execute(
                    "INSERT INTO device (id, node_id, secret_backend, created_at_ms, receiver_name,
                     local_network_explained, old_keys_left)
                     VALUES (1, ?1, 'keychain', 7, NULL, 1, 0)",
                    [[9u8; 32]],
                )
                .unwrap();
        }
        let look = SqliteStore::open(&path).unwrap().version_look().unwrap();
        assert_eq!(look, VersionLook::default());
        assert!(look.looking && !look.looking_told);
    }

    #[test]
    fn what_is_kept_comes_back_across_a_reopen() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.db");
        let mut look = VersionLook {
            looking: false,
            looking_told: true,
            next_look_ms: 7,
            last_good_ms: Some(5),
            failing_since_ms: Some(6),
            failure: Some(LookFailure::Refused),
            newest: None,
            security: Some("0.3.0".into()),
            new_release_key: Some("00112233AABBCCDD".into()),
            security_told: Some("0.2.9".into()),
            ..VersionLook::default()
        };
        look.serials.insert("2DDEFEB70EAC0BBB".into(), 9);
        look.serials.insert("971E8F0C5AA6A65E".into(), 1);
        SqliteStore::open(&path)
            .unwrap()
            .set_version_look(&look)
            .unwrap();
        assert_eq!(
            SqliteStore::open(&path).unwrap().version_look().unwrap(),
            look
        );
        // Written again, a serial gone is gone.
        look.serials.remove("971E8F0C5AA6A65E");
        let store = SqliteStore::open(&path).unwrap();
        store.set_version_look(&look).unwrap();
        assert_eq!(store.version_look().unwrap(), look);
    }
}
