use super::*;
use crate::cipher::CipherId;
use crate::secrets::MemorySecretStore;
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{Device, JoinedStick, MemoryStore, OwnedStick, SecretBackend};

fn stick(seed: u8) -> StickId {
    StickSigningKey::from_bytes(&[seed; 32]).stick_id()
}

/// A device with an older node key, one stick of its own with its own node
/// key, and one received stick.
fn device_store() -> MemoryStore {
    let store = MemoryStore::new();
    store
        .create_device(&Device {
            node_id: Some(crate::ticket::NodeId([5; 32])),
            secret_backend: SecretBackend::Keychain,
            created_at_ms: 0,
        })
        .unwrap();
    let mut owned = OwnedStick::new(stick(1), "s", "/s", CipherId::None, 0);
    owned.node_id = Some(crate::ticket::NodeId([6; 32]));
    store.add_owned_stick(&owned).unwrap();
    store
        .add_joined_stick(&JoinedStick::new(stick(2), "/r", 0))
        .unwrap();
    store
}

/// The login keychain as the device left it.
fn old_store(to_move: &ToMove) -> MemorySecretStore {
    let secrets = MemorySecretStore::new();
    for (index, name) in to_move.required.iter().enumerate() {
        secrets.put(name, &[index as u8; 32]).unwrap();
    }
    secrets
}

#[test]
fn every_secret_the_database_names_is_listed() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    assert_eq!(
        to_move.required,
        vec![
            SecretName::NodeKey,
            SecretName::StickSigningKey(stick(1)),
            SecretName::StickKey(stick(1)),
            SecretName::StickNodeKey(stick(1)),
            SecretName::Ticket(stick(2)),
        ]
    );
    assert_eq!(to_move.optional, vec![SecretName::ReaderKey(stick(2))]);
}

#[test]
fn every_secret_is_copied_and_read_back_and_a_second_run_writes_nothing() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let old = old_store(&to_move);
    old.put(&SecretName::ReaderKey(stick(2)), &[9; 32]).unwrap();
    let new = MemorySecretStore::new();
    assert_eq!(copy_secrets(&old, &new, &to_move).unwrap(), 6);
    all_there(&new, &to_move).unwrap();
    for name in to_move.required.iter().chain(&to_move.optional) {
        assert_eq!(new.get(name).unwrap(), old.get(name).unwrap(), "{name:?}");
    }
    // Stopped after copying, run again: nothing more to write.
    assert_eq!(copy_secrets(&old, &new, &to_move).unwrap(), 0);
    delete_old(&old, &to_move).unwrap();
    for name in to_move.required.iter().chain(&to_move.optional) {
        assert_eq!(old.get(name).unwrap(), None);
    }
    all_there(&new, &to_move).unwrap();
}

#[test]
fn a_missing_key_stops_it_before_anything_is_written() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let old = old_store(&to_move);
    old.delete(&SecretName::StickKey(stick(1))).unwrap();
    let new = MemorySecretStore::new();
    assert!(matches!(
        copy_secrets(&old, &new, &to_move),
        Err(MoveError::Missing(_))
    ));
    for name in &to_move.required {
        assert_eq!(new.get(name).unwrap(), None, "nothing written");
    }
}

#[test]
fn a_key_already_there_with_another_value_is_never_overwritten() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let old = old_store(&to_move);
    let new = MemorySecretStore::new();
    new.put(&SecretName::NodeKey, &[77; 32]).unwrap();
    assert!(matches!(
        copy_secrets(&old, &new, &to_move),
        Err(MoveError::Differs(_))
    ));
    assert_eq!(new.get(&SecretName::NodeKey).unwrap(), Some(vec![77; 32]));
    assert_eq!(new.get(&SecretName::Ticket(stick(2))).unwrap(), None);
}

#[test]
fn the_old_copies_go_only_when_the_new_store_has_every_one() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let half = MemorySecretStore::new();
    half.put(&SecretName::NodeKey, &[0; 32]).unwrap();
    assert!(matches!(
        all_there(&half, &to_move),
        Err(MoveError::Missing(_))
    ));
}

/// A store that refuses one name ("Deny"), or forgets what it is given.
struct Refusing {
    inner: MemorySecretStore,
    refused: Option<SecretName>,
    forgets: bool,
}

impl SecretStore for Refusing {
    fn describe(&self) -> String {
        "refusing".to_owned()
    }
    fn durability(&self) -> crate::secrets::Durability {
        crate::secrets::Durability::Durable
    }
    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        if self.forgets {
            return Ok(());
        }
        self.inner.put(name, secret)
    }
    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        if self.refused.as_ref() == Some(name) {
            return Err(SecretError::AccessDenied {
                store: "refusing".to_owned(),
                reason: "denied".to_owned(),
            });
        }
        self.inner.get(name)
    }
    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.inner.delete(name)
    }
}

#[test]
fn a_refusal_on_one_key_writes_nothing() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let old = Refusing {
        inner: old_store(&to_move),
        refused: Some(SecretName::Ticket(stick(2))),
        forgets: false,
    };
    let new = MemorySecretStore::new();
    assert!(matches!(
        copy_secrets(&old, &new, &to_move),
        Err(MoveError::Secret(_))
    ));
    assert_eq!(new.get(&SecretName::NodeKey).unwrap(), None);
}

#[test]
fn a_store_that_does_not_keep_what_it_is_given_is_caught() {
    let to_move = secrets_to_move(&device_store()).unwrap();
    let old = old_store(&to_move);
    let new = Refusing {
        inner: MemorySecretStore::new(),
        refused: None,
        forgets: true,
    };
    assert!(matches!(
        copy_secrets(&old, &new, &to_move),
        Err(MoveError::NotKept(_))
    ));
}

#[test]
fn a_move_records_the_new_store_and_a_later_start_deletes_the_old_copies() {
    let store = device_store();
    let to_move = secrets_to_move(&store).unwrap();
    let old = old_store(&to_move);
    let new = MemorySecretStore::new();
    assert_eq!(move_keys(&store, &old, &new).unwrap(), 5);
    assert_eq!(
        store.device().unwrap().unwrap().secret_backend,
        SecretBackend::ProtectedKeychain
    );
    assert!(store.old_keys_left().unwrap());
    assert!(
        old.get(&SecretName::NodeKey).unwrap().is_some(),
        "kept until a later start"
    );
    assert!(finish_move(&store, &old, &new).unwrap());
    assert!(!store.old_keys_left().unwrap());
    assert_eq!(old.get(&SecretName::NodeKey).unwrap(), None);
    assert!(new.get(&SecretName::NodeKey).unwrap().is_some());
    // Nothing left to do at the next start.
    assert!(finish_move(&store, &old, &new).unwrap());
}

#[test]
fn a_failed_move_records_nothing() {
    let store = device_store();
    let to_move = secrets_to_move(&store).unwrap();
    let old = old_store(&to_move);
    old.delete(&SecretName::Ticket(stick(2))).unwrap();
    let new = MemorySecretStore::new();
    assert!(move_keys(&store, &old, &new).is_err());
    assert_eq!(
        store.device().unwrap().unwrap().secret_backend,
        SecretBackend::Keychain
    );
    assert!(!store.old_keys_left().unwrap());
}

#[test]
fn the_old_copies_stay_if_the_new_store_lacks_one() {
    let store = device_store();
    let to_move = secrets_to_move(&store).unwrap();
    let old = old_store(&to_move);
    let new = MemorySecretStore::new();
    move_keys(&store, &old, &new).unwrap();
    new.delete(&SecretName::StickKey(stick(1))).unwrap();
    assert!(finish_move(&store, &old, &new).is_err());
    assert!(store.old_keys_left().unwrap());
    assert!(old.get(&SecretName::StickKey(stick(1))).unwrap().is_some());
}
