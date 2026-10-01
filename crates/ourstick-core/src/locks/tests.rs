use super::*;
use crate::cipher::CipherId;
use crate::stick_id::StickSigningKey;
use crate::store::MemoryStore;
use crate::ticket::NodeId;

/// A stick of `seed`, with its own node key if `own_node_key`.
fn stick(store: &MemoryStore, seed: u8, name: &str, own_node_key: bool) -> OwnedStick {
    let stick = OwnedStick {
        node_id: own_node_key.then_some(NodeId([seed; 32])),
        ..OwnedStick::new(
            StickSigningKey::from_bytes(&[seed; 32]).stick_id(),
            name,
            format!("/folder/{seed}"),
            CipherId::XChaCha20Poly1305,
            1,
        )
    };
    store.add_owned_stick(&stick).unwrap();
    stick
}

#[test]
fn sticks_with_their_own_node_keys_are_shared_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let one = stick(&store, 1, "one", true);
    let two = stick(&store, 2, "two", true);
    let (_first, _) = lock_for_sharing(dir.path(), &store, &one.stick_id).unwrap();
    let (_second, _) = lock_for_sharing(dir.path(), &store, &two.stick_id).unwrap();
    // The same stick twice: refused, by name.
    match lock_for_sharing(dir.path(), &store, &one.stick_id) {
        Err(LockError::StickInUse { name }) => assert_eq!(name, "one"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn older_sticks_are_shared_one_at_a_time_and_the_refusal_names_the_shared_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let photos = stick(&store, 1, "Photos", false);
    let music = stick(&store, 2, "Music", false);
    let new = stick(&store, 3, "New", true);
    let (held, _) = lock_for_sharing(dir.path(), &store, &photos.stick_id).unwrap();
    match lock_for_sharing(dir.path(), &store, &music.stick_id) {
        Err(LockError::NodeKeyInUse { shared }) => assert_eq!(shared.as_deref(), Some("Photos")),
        other => panic!("{other:?}"),
    }
    // The refusal let Music's own lock go; a stick with its own key is free.
    let (_new, _) = lock_for_sharing(dir.path(), &store, &new.stick_id).unwrap();
    drop(held);
    let (_music, _) = lock_for_sharing(dir.path(), &store, &music.stick_id).unwrap();
}

#[test]
fn the_row_is_read_again_under_the_lock_and_a_gone_stick_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let missing = StickSigningKey::from_bytes(&[9; 32]).stick_id();
    assert!(matches!(
        lock_for_sharing(dir.path(), &store, &missing),
        Err(LockError::Store(StoreError::StickNotFound))
    ));
    let one = stick(&store, 1, "one", true);
    store
        .move_owned_stick(&one.stick_id, Path::new("/elsewhere"))
        .unwrap();
    let (_lock, read) = lock_for_sharing(dir.path(), &store, &one.stick_id).unwrap();
    assert_eq!(read.root_path, Path::new("/elsewhere"));
}

#[test]
fn the_key_file_and_setup_locks_are_one_program_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let key_file = key_file_lock(dir.path()).unwrap();
    assert!(matches!(
        key_file_lock(dir.path()),
        Err(LockError::KeyFileInUse)
    ));
    let setup = setup_lock(dir.path()).unwrap();
    assert!(matches!(setup_lock(dir.path()), Err(LockError::SetupInUse)));
    drop((key_file, setup));
    key_file_lock(dir.path()).unwrap();
    setup_lock(dir.path()).unwrap();
}

#[test]
fn the_refusal_names_no_stick_when_it_cannot_tell() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let older = stick(&store, 1, "Older", false);
    // Held by a program between its two locks, or one that holds only it.
    let _node_key = try_take(dir.path().join("node-key.lock")).unwrap().unwrap();
    match lock_for_sharing(dir.path(), &store, &older.stick_id) {
        Err(error @ LockError::NodeKeyInUse { shared: None }) => {
            assert!(error
                .to_string()
                .starts_with("another older stick is being shared"));
        }
        other => panic!("{other:?}"),
    }
}
