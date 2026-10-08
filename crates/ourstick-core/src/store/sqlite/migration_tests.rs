//! Migrations from version 15 on, on databases as earlier versions left them.

use super::*;
use crate::store::StickKind;

#[test]
fn migration_15_keeps_every_stick_shared_and_live() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    {
        // A database as version 14 left it.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..14] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 14).unwrap();
        connection
            .execute(
                "INSERT INTO owned_sticks VALUES (?1, 'OurStick', '/d', 1, 0, NULL, 3, NULL, 0, NULL, 'app')",
                [stick.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let read = store.owned_stick(&stick).unwrap().unwrap();
    assert_eq!(read.kind, StickKind::App);
    assert!(read.sharing_wanted);
    assert_eq!(read.image_path, None);
    assert!(store.removing_sticks().unwrap().is_empty());
    assert!(store.retired_sticks().unwrap().is_empty());
}

#[test]
fn migration_16_keeps_every_claim_and_takes_scratch() {
    use crate::store::{ClaimKind, SpaceClaim};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[2; 32]).stick_id();
    {
        // A database as version 15 left it, with a download's claim.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..15] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 15).unwrap();
        connection
            .execute(
                "INSERT INTO space_claims VALUES (?1, 'download', 'dev:7', 42)",
                [stick.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let download = SpaceClaim {
        stick_id: stick,
        kind: ClaimKind::Download,
        disk: "dev:7".to_owned(),
        bytes: 42,
    };
    assert_eq!(
        store.space_claims().unwrap(),
        std::slice::from_ref(&download)
    );
    let scratch = SpaceClaim {
        kind: ClaimKind::Scratch,
        bytes: 9,
        ..download
    };
    store.claim_space(&mut |_| vec![scratch.clone()]).unwrap();
    assert_eq!(store.space_claims().unwrap().len(), 2);
}

#[test]
fn migration_17_keeps_what_get_received_as_get_s() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    {
        // A database as version 16 left it, with a stick `get` received.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..16] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 16).unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks (stick_id, output_dir, joined_at_ms) VALUES (?1, '/d/x', 4)",
                [stick.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let read = store.joined_stick(&stick).unwrap().unwrap();
    assert_eq!(read.kind, StickKind::Cli);
    assert_eq!(read.name, None);
    assert!(!read.paused);
    assert!(store.forgetting_joined_sticks().unwrap().is_empty());
}

#[test]
fn migration_18_saves_every_received_file_at_its_own_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    {
        // A database as version 17 left it, with two received files.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..17] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 17).unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks (stick_id, output_dir, joined_at_ms) VALUES (?1, '/d/x', 4)",
                [stick.to_bytes()],
            )
            .unwrap();
        for file in ["Trip/a.jpg", "b.txt"] {
            connection
                .execute(
                    "INSERT INTO downloaded_files VALUES (?1, ?2, 1, 2, 3, zeroblob(32))",
                    rusqlite::params![stick.to_bytes(), file],
                )
                .unwrap();
        }
    }
    let store = SqliteStore::open(&path).unwrap();
    let files = store.downloaded_files(&stick).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.saved_as == file.path));
    assert!(store.saved_elsewhere(&stick).unwrap().is_empty());
}

#[test]
fn migration_19_leaves_nothing_out() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    {
        // A database as version 18 left it, with a received stick.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..18] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 18).unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks (stick_id, output_dir, joined_at_ms) VALUES (?1, '/d/x', 4)",
                [stick.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert!(store.left_out(&stick).unwrap().is_empty());
    store.set_left_out(&stick, &["Films".to_owned()]).unwrap();
    assert_eq!(store.left_out(&stick).unwrap(), ["Films"]);
}

#[test]
fn migration_20_starts_every_counter_and_list_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let stick = crate::stick_id::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    {
        // A database as version 19 left it, with a device and a received
        // stick.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..19] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 19).unwrap();
        connection
            .execute(
                "INSERT INTO device (id, node_id, secret_backend, created_at_ms)
                 VALUES (1, ?1, 'secret-service', 1)",
                [[9u8; 32]],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO joined_sticks (stick_id, output_dir, joined_at_ms) VALUES (?1, '/d/x', 4)",
                [stick.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.receiver_name().unwrap(), None);
    assert_eq!(store.next_report_counter(&stick, 5).unwrap(), 5);
    store.set_receiver_name(Some("Μαρία")).unwrap();
    assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    // The new table holds a line, and its checks hold.
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let insert = |percent: Option<i64>, state: &str| {
        connection.execute(
            "INSERT INTO reader_reports (stick_id, reader_id, counter, seq, state, percent,
             everything, kind, first_ms, received_ms) VALUES (?1, ?2, 1, 1, ?3, ?4, 1, 'linux', 5, 5)",
            rusqlite::params![[4u8; 32], [1u8; 32], state, percent],
        )
    };
    // Its stick must be shared here.
    assert!(insert(Some(5), "downloading").is_err());
    connection
        .execute(
            "INSERT INTO owned_sticks (stick_id, name, root_path, cipher, created_at_ms)
             VALUES (?1, 'x', '/x', 1, 1)",
            [[4u8; 32]],
        )
        .unwrap();
    assert!(insert(None, "downloading").is_err());
    assert!(insert(Some(100), "downloading").is_err());
    assert!(insert(Some(5), "up_to_date").is_err());
    insert(Some(5), "downloading").unwrap();
}

#[test]
fn migration_21_removes_cache_claims_and_keeps_downloads() {
    use crate::store::ClaimKind;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let one = crate::stick_id::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    let two = crate::stick_id::StickSigningKey::from_bytes(&[4; 32]).stick_id();
    {
        // A database as version 20 left it: a sharer's cache claim, and a
        // download's.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..20] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 20).unwrap();
        connection
            .execute(
                "INSERT INTO space_claims VALUES (?1, 'cache', 'dev:7', 99)",
                [one.to_bytes()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO space_claims VALUES (?1, 'download', 'dev:7', 42)",
                [two.to_bytes()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let claims = store.space_claims().unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, ClaimKind::Download);
}

#[test]
fn migration_23_keeps_the_device_and_takes_the_protected_keychain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    {
        // A Mac's database as version 22 left it.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..22] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 22).unwrap();
        connection
            .execute(
                "INSERT INTO device (id, node_id, secret_backend, created_at_ms, receiver_name,
                 local_network_explained) VALUES (1, ?1, 'keychain', 7, 'Μαρία', 1)",
                [[9u8; 32]],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let device = store.device().unwrap().unwrap();
    assert_eq!(device.secret_backend, crate::store::SecretBackend::Keychain);
    assert_eq!(device.created_at_ms, 7);
    assert_eq!(device.node_id.map(|id| id.0), Some([9; 32]));
    assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    assert!(store.local_network_explained().unwrap());
    // Old copies only with the new store.
    let connection = Connection::open(&path).unwrap();
    assert!(connection
        .execute("UPDATE device SET old_keys_left = 1", [])
        .is_err());
    // The new value is allowed.
    connection
        .execute(
            "UPDATE device SET secret_backend = 'protected-keychain'",
            [],
        )
        .unwrap();
    drop(connection);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.device().unwrap().unwrap().secret_backend,
        crate::store::SecretBackend::ProtectedKeychain
    );
}
