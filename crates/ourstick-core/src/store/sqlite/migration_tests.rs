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
