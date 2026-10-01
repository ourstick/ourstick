use super::*;
use crate::cipher::CipherId;
use crate::store::StickKind;

#[test]
fn fresh_database_has_current_schema() {
    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 13);
}

#[test]
fn migration_2_moves_the_floor_out_and_forgets_what_folders_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let seen = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    let unseen = crate::stick_id::StickSigningKey::from_bytes(&[2; 32]).stick_id();
    {
        // A database as version 1 left it.
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(MIGRATIONS[0]).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks VALUES (?1, '/tmp/a', 7, ?2, 10)",
                params![seen.to_bytes(), [3u8; 32].to_vec()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks VALUES (?1, '/tmp/b', NULL, NULL, 20)",
                params![unseen.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(
        store.record_floor(&seen).unwrap(),
        Some(ManifestVersion {
            seq: 7,
            hash: [3; 32]
        })
    );
    assert_eq!(store.record_floor(&unseen).unwrap(), None);
    assert_eq!(
        store.joined_sticks().unwrap(),
        vec![
            JoinedStick::new(seen, "/tmp/a", 10),
            JoinedStick::new(unseen, "/tmp/b", 20),
        ]
    );
}

#[test]
fn migration_6_keeps_the_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..5] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 5).unwrap();
        connection
            .execute(
                "INSERT INTO device VALUES (1, ?1, 'key-file', '/k.oskf', 7)",
                [[9u8; 32].to_vec()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.device().unwrap(),
        Some(Device {
            node_id: Some(NodeId([9; 32])),
            secret_backend: SecretBackend::KeyFile("/k.oskf".into()),
            created_at_ms: 7,
        })
    );
}

#[test]
fn migration_7_keeps_the_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..6] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 6).unwrap();
        connection
            .execute(
                "INSERT INTO device VALUES (1, ?1, 'keychain', NULL, 7)",
                [[9u8; 32].to_vec()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.device().unwrap(),
        Some(Device {
            node_id: Some(NodeId([9; 32])),
            secret_backend: SecretBackend::Keychain,
            created_at_ms: 7,
        })
    );
}

#[test]
fn reopening_keeps_data_and_does_not_migrate_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    {
        let store = SqliteStore::open(&path).unwrap();
        store
            .add_joined_stick(&JoinedStick::new(stick_id, "/tmp/out", 1))
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(store.joined_stick(&stick_id).unwrap().is_some());
}

#[test]
fn newer_database_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    drop(SqliteStore::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();

    assert!(matches!(
        SqliteStore::open(&path),
        Err(StoreError::NewerDatabase { found, supported })
            if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION
    ));
    let version: i64 = user_version(&Connection::open(&path).unwrap()).unwrap();
    assert_eq!(version, SCHEMA_VERSION + 1);
}

#[test]
fn file_database_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("state.sqlite")).unwrap();
    let mode: String = store
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
}

#[test]
fn schema_rejects_bad_rows_even_without_the_code() {
    // CHECK constraints guard the data if a bug or a hand edit bypasses the code.
    let store = SqliteStore::open_in_memory().unwrap();
    let connection = store.connection();
    let bad_inserts = [
        "INSERT INTO device VALUES (2, zeroblob(32), 'secret-service', NULL, 0)",
        "INSERT INTO device VALUES (1, zeroblob(31), 'secret-service', NULL, 0)",
        "INSERT INTO device VALUES (1, zeroblob(32), 'plaintext-file', NULL, 0)",
        "INSERT INTO device VALUES (1, zeroblob(32), 'keychain', '/k', 0)",
        "INSERT INTO device VALUES (1, zeroblob(32), 'credential-manager', '/k', 0)",
        "INSERT INTO device VALUES (1, zeroblob(32), 'key-file', NULL, 0)",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, -1, NULL, 0, NULL, NULL, 0, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, zeroblob(10), 0, NULL, NULL, 0, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, 0, NULL, 0, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, 'o', 0, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, 'OO', 0, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, NULL, 2, NULL, 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, NULL, 0, zeroblob(31), 'cli')",
        "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, NULL, 0, NULL, 'phone')",
        "INSERT INTO joined_sticks VALUES (zeroblob(32), '/', 5, NULL, 0)",
        "INSERT INTO joined_sticks VALUES (zeroblob(32), '/', 'five', zeroblob(32), 0)",
        "INSERT INTO record_floors VALUES (zeroblob(32), NULL, zeroblob(32))",
        "INSERT INTO record_floors VALUES (zeroblob(32), -1, zeroblob(32))",
        "INSERT INTO record_floors VALUES (zeroblob(32), 1, zeroblob(31))",
        // No owned stick with that id (foreign key), and bad values.
        "INSERT INTO owned_files VALUES (randomblob(32), 'a', 1, 1, 1, zeroblob(32), NULL)",
    ];
    connection
        .execute(
            "INSERT INTO owned_sticks VALUES (zeroblob(32), 'n', '/', 1, 0, NULL, 0, NULL, NULL, 0, NULL, 'cli')",
            [],
        )
        .unwrap();
    let bad_inserts = [
        bad_inserts.as_slice(),
        &[
            "INSERT INTO owned_files VALUES (zeroblob(32), 'a', -1, 1, 1, zeroblob(32), NULL)",
            "INSERT INTO owned_files VALUES (zeroblob(32), 'a', 1, 1, 1, zeroblob(31), NULL)",
            "INSERT INTO owned_files VALUES (zeroblob(32), NULL, 1, 1, 1, zeroblob(32), NULL)",
            "INSERT INTO owned_files VALUES (zeroblob(32), 'a', 1, 1, 1, zeroblob(32), zeroblob(31))",
            "INSERT INTO downloaded_files VALUES (randomblob(32), 'a', 1, 1, 1, zeroblob(32))",
        ],
    ]
    .concat();
    for sql in bad_inserts {
        assert!(connection.execute(sql, []).is_err(), "accepted: {sql}");
    }
    // The good row the bad ones differ from.
    for plaintext_hash in ["NULL", "zeroblob(32)"] {
        connection
            .execute(
                &format!(
                    "INSERT OR REPLACE INTO owned_files
                     VALUES (zeroblob(32), 'a', 1, 1, 1, zeroblob(32), {plaintext_hash})"
                ),
                [],
            )
            .unwrap();
    }
}

#[test]
fn two_connections_cannot_both_record_the_same_seq() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let owner = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]);
    let first = SqliteStore::open(&path).unwrap();
    let second = SqliteStore::open(&path).unwrap();
    first
        .add_owned_stick(&OwnedStick::new(
            owner.stick_id(),
            "s",
            "/tmp/s",
            CipherId::XChaCha20Poly1305,
            1,
        ))
        .unwrap();

    first
        .save_signed_record(&owner.sign_record(10, [1; 32]), false)
        .unwrap();
    assert!(matches!(
        second.save_signed_record(&owner.sign_record(10, [2; 32]), false),
        Err(StoreError::NotNewer {
            seq: 10,
            stored: 10
        })
    ));
}

#[test]
fn migration_11_keeps_the_device_and_tells_app_sticks_from_cli_ones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let drive = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    let folder = crate::stick_id::StickSigningKey::from_bytes(&[2; 32]).stick_id();
    {
        // A database as version 10 left it.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..10] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 10).unwrap();
        connection
            .execute(
                "INSERT INTO device VALUES (1, ?1, 'key-file', '/k.oskf', 7)",
                [[9u8; 32].to_vec()],
            )
            .unwrap();
        for (stick, drive_size) in [(drive, Some(1_000_000i64)), (folder, None)] {
            connection
                .execute(
                    "INSERT INTO owned_sticks VALUES (?1, 'n', ?2, 1, 0, NULL, 0, ?3, NULL, 0)",
                    params![
                        stick.to_bytes(),
                        format!("/{}", drive_size.is_some()),
                        drive_size
                    ],
                )
                .unwrap();
        }
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.device().unwrap().unwrap().node_id,
        Some(NodeId([9; 32])),
        "an existing device keeps its node key (rule 9)"
    );
    let kinds: Vec<_> = store
        .owned_sticks()
        .unwrap()
        .into_iter()
        .map(|stick| (stick.stick_id, stick.kind, stick.node_id))
        .collect();
    assert!(kinds.contains(&(drive, StickKind::App, None)));
    assert!(kinds.contains(&(folder, StickKind::Cli, None)));
}

#[test]
fn the_database_itself_refuses_bad_linked_entries() {
    let store = SqliteStore::open_in_memory().unwrap();
    let stick = OwnedStick::new(
        crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id(),
        "s",
        "/s",
        CipherId::XChaCha20Poly1305,
        1,
    );
    store.add_owned_stick(&stick).unwrap();
    let connection = store.connection();
    let insert = |stick_id: Vec<u8>, name: &str, path: &str| {
        connection.execute(
            "INSERT INTO linked_entries VALUES (?1, ?2, ?3, 1, NULL, 0)",
            params![stick_id, name, path],
        )
    };
    let id = stick.stick_id.to_bytes().to_vec();
    insert(id.clone(), "Films", "/media/Films").unwrap();
    // A stick that is not there, a name with a slash, an empty one, no path.
    assert!(insert(vec![9; 32], "x", "/x").is_err());
    assert!(insert(id.clone(), "a/b", "/x").is_err());
    assert!(insert(id.clone(), "", "/x").is_err());
    assert!(insert(id, "y", "").is_err());
}
