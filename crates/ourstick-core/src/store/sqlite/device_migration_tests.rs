//! Migrations that make the `device` table again, on databases as earlier
//! versions left them.

use super::super::*;

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

#[test]
fn migration_28_keeps_the_device_and_takes_the_android_keystore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    {
        // A Mac's database as version 27 left it, its old keys still there.
        let connection = Connection::open(&path).unwrap();
        for migration in &MIGRATIONS[..27] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 27).unwrap();
        connection
            .execute(
                "INSERT INTO device (id, node_id, secret_backend, created_at_ms, receiver_name,
                 local_network_explained, old_keys_left)
                 VALUES (1, ?1, 'protected-keychain', 7, 'Μαρία', 1, 1)",
                [[9u8; 32]],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let device = store.device().unwrap().unwrap();
    assert_eq!(
        device.secret_backend,
        crate::store::SecretBackend::ProtectedKeychain
    );
    assert_eq!(device.created_at_ms, 7);
    assert_eq!(device.node_id.map(|id| id.0), Some([9; 32]));
    assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    assert!(store.local_network_explained().unwrap());
    assert!(store.old_keys_left().unwrap());
    // The new value is allowed, without old copies.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE device SET secret_backend = 'android-keystore', old_keys_left = 0",
            [],
        )
        .unwrap();
    assert!(connection
        .execute("UPDATE device SET old_keys_left = 1", [])
        .is_err());
    drop(connection);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.device().unwrap().unwrap().secret_backend,
        crate::store::SecretBackend::AndroidKeystore
    );
}
