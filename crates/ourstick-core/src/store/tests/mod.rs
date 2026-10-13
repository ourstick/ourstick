//! Behaviour every [`Store`] must have. Each scenario runs against both
//! [`MemoryStore`] and [`SqliteStore`], so the two cannot drift apart.

use super::*;
use crate::cipher::CipherId;
use crate::record::ManifestRecord;
use crate::stick_id::StickSigningKey;
use crate::ticket::NodeId;

fn owner(seed: u8) -> StickSigningKey {
    StickSigningKey::from_bytes(&[seed; 32])
}

fn device() -> Device {
    Device {
        node_id: Some(NodeId([9; 32])),
        secret_backend: SecretBackend::SecretService,
        created_at_ms: 1_700_000_000_000,
    }
}

fn owned(seed: u8, created_at_ms: u64) -> OwnedStick {
    OwnedStick::new(
        owner(seed).stick_id(),
        format!("stick {seed}"),
        format!("/home/user/stick-{seed}"),
        CipherId::XChaCha20Poly1305,
        created_at_ms,
    )
}

/// `path` (written `/a/b`) as an absolute path on this system: on Windows
/// a path needs a drive to be absolute.
fn absolute(path: &str) -> std::path::PathBuf {
    if cfg!(windows) && path.starts_with('/') {
        format!("C:{}", path.replace('/', "\\")).into()
    } else {
        path.into()
    }
}

fn joined(seed: u8, joined_at_ms: u64) -> JoinedStick {
    JoinedStick::new(
        owner(seed).stick_id(),
        format!("/tmp/out-{seed}"),
        joined_at_ms,
    )
}

fn record(seed: u8, seq: u64, hash: u8) -> ManifestRecord {
    ManifestRecord {
        stick_id: owner(seed).stick_id(),
        seq,
        manifest_hash: [hash; 32],
    }
}

fn device_is_created_once(store: &dyn Store) {
    assert_eq!(store.device().unwrap(), None);
    store.create_device(&device()).unwrap();
    assert_eq!(store.device().unwrap(), Some(device()));

    let replacement = Device {
        node_id: Some(NodeId([8; 32])),
        ..device()
    };
    assert!(matches!(
        store.create_device(&replacement),
        Err(StoreError::DeviceExists)
    ));
    assert_eq!(store.device().unwrap(), Some(device()));
}

fn device_with_key_file_roundtrips(store: &dyn Store) {
    let device = Device {
        secret_backend: SecretBackend::KeyFile("/home/user/.ourstick/keys.oskf".into()),
        ..device()
    };
    store.create_device(&device).unwrap();
    assert_eq!(store.device().unwrap(), Some(device));
}

fn device_with_keychain_roundtrips(store: &dyn Store) {
    let device = Device {
        secret_backend: SecretBackend::Keychain,
        ..device()
    };
    store.create_device(&device).unwrap();
    assert_eq!(store.device().unwrap(), Some(device));
}

fn device_with_credential_manager_roundtrips(store: &dyn Store) {
    let device = Device {
        secret_backend: SecretBackend::CredentialManager,
        ..device()
    };
    store.create_device(&device).unwrap();
    assert_eq!(store.device().unwrap(), Some(device));
}

fn a_moved_stick_keeps_everything_but_its_folder(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    assert!(matches!(
        store.move_owned_stick(&stick_id, Path::new("/Volumes/OurStick 1")),
        Err(StoreError::StickNotFound)
    ));
    store.add_owned_stick(&owned(1, 10)).unwrap();
    store
        .move_owned_stick(&stick_id, Path::new("/Volumes/OurStick 1"))
        .unwrap();
    assert_eq!(
        store.owned_stick(&stick_id).unwrap(),
        Some(OwnedStick {
            root_path: PathBuf::from("/Volumes/OurStick 1"),
            ..owned(1, 10)
        })
    );
}

fn a_stick_keeps_its_drive_letter(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    assert!(matches!(
        store.set_drive_letter(&stick_id, Some('O')),
        Err(StoreError::StickNotFound)
    ));
    store.add_owned_stick(&owned(1, 10)).unwrap();
    store.set_drive_letter(&stick_id, Some('O')).unwrap();
    assert_eq!(
        store.owned_stick(&stick_id).unwrap().unwrap().drive_letter,
        Some('O')
    );
    for bad in ['o', '1', 'Ω'] {
        assert!(
            matches!(
                store.set_drive_letter(&stick_id, Some(bad)),
                Err(StoreError::Inconsistent(_))
            ),
            "{bad}"
        );
    }
    store.set_drive_letter(&stick_id, None).unwrap();
    assert_eq!(store.owned_stick(&stick_id).unwrap(), Some(owned(1, 10)));
}

fn owned_sticks_roundtrip_in_order(store: &dyn Store) {
    store.add_owned_stick(&owned(2, 20)).unwrap();
    store.add_owned_stick(&owned(1, 10)).unwrap();
    assert_eq!(
        store.owned_stick(&owner(1).stick_id()).unwrap(),
        Some(owned(1, 10))
    );
    assert_eq!(store.owned_stick(&owner(3).stick_id()).unwrap(), None);
    assert_eq!(
        store.owned_sticks().unwrap(),
        vec![owned(1, 10), owned(2, 20)]
    );
    assert!(matches!(
        store.add_owned_stick(&owned(1, 99)),
        Err(StoreError::StickExists)
    ));
}

fn signed_records_only_move_forward(store: &dyn Store) {
    store.add_owned_stick(&owned(1, 10)).unwrap();
    let first = owner(1).sign_record(100, [1; 32]);
    store.save_signed_record(&first, false).unwrap();

    let stored = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert_eq!(stored.last_signed_seq, 100);
    assert_eq!(stored.latest_record, Some(first.clone()));
    assert!(!stored.latest_partial);

    for seq in [100, 99, 0] {
        assert!(matches!(
            store.save_signed_record(&owner(1).sign_record(seq, [2; 32]), false),
            Err(StoreError::NotNewer { stored: 100, .. })
        ));
    }
    let unchanged = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert_eq!(unchanged.latest_record, Some(first));

    store
        .save_signed_record(&owner(1).sign_record(101, [3; 32]), true)
        .unwrap();
    let partial = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert_eq!(partial.last_signed_seq, 101);
    assert!(partial.latest_partial);
    store
        .save_signed_record(&owner(1).sign_record(102, [4; 32]), false)
        .unwrap();
    let complete = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert!(!complete.latest_partial);
}

fn signed_record_for_unknown_stick_fails(store: &dyn Store) {
    assert!(matches!(
        store.save_signed_record(&owner(1).sign_record(1, [1; 32]), false),
        Err(StoreError::StickNotFound)
    ));
}

fn owned_stick_must_be_consistent(store: &dyn Store) {
    let signed_without_record = OwnedStick {
        last_signed_seq: 5,
        ..owned(1, 10)
    };
    let record_of_other_stick = OwnedStick {
        last_signed_seq: 5,
        latest_record: Some(owner(2).sign_record(5, [1; 32])),
        ..owned(1, 10)
    };
    let record_with_other_seq = OwnedStick {
        last_signed_seq: 5,
        latest_record: Some(owner(1).sign_record(6, [1; 32])),
        ..owned(1, 10)
    };
    for stick in [
        signed_without_record,
        record_of_other_stick,
        record_with_other_seq,
    ] {
        assert!(matches!(
            store.add_owned_stick(&stick),
            Err(StoreError::Inconsistent(_))
        ));
    }
    let consistent = OwnedStick {
        last_signed_seq: 5,
        latest_record: Some(owner(1).sign_record(5, [1; 32])),
        ..owned(1, 10)
    };
    store.add_owned_stick(&consistent).unwrap();
    assert_eq!(
        store.owned_stick(&owner(1).stick_id()).unwrap(),
        Some(consistent)
    );
}

fn joined_sticks_roundtrip_in_order(store: &dyn Store) {
    store.add_joined_stick(&joined(2, 20)).unwrap();
    store.add_joined_stick(&joined(1, 10)).unwrap();
    assert_eq!(
        store.joined_stick(&owner(1).stick_id()).unwrap(),
        Some(joined(1, 10))
    );
    assert_eq!(store.joined_stick(&owner(3).stick_id()).unwrap(), None);
    assert_eq!(
        store.joined_sticks().unwrap(),
        vec![joined(1, 10), joined(2, 20)]
    );
    assert!(matches!(
        store.add_joined_stick(&joined(1, 99)),
        Err(StoreError::StickExists)
    ));
}

fn values_beyond_i64_are_refused(store: &dyn Store) {
    let too_big = MAX_STORED_SEQ + 1;
    store.add_owned_stick(&owned(1, 10)).unwrap();
    assert!(matches!(
        store.save_signed_record(&owner(1).sign_record(too_big, [1; 32]), false),
        Err(StoreError::SeqOutOfRange(_))
    ));
    assert!(matches!(
        store.accept_record(&record(1, too_big, 1)),
        Err(StoreError::SeqOutOfRange(_))
    ));
    assert!(matches!(
        store.add_owned_stick(&owned(2, too_big)),
        Err(StoreError::TimeOutOfRange(_))
    ));
    // The largest storable seq still works.
    store
        .save_signed_record(&owner(1).sign_record(MAX_STORED_SEQ, [1; 32]), false)
        .unwrap();
}

#[cfg(unix)]
fn non_utf8_paths_are_refused(store: &dyn Store) {
    use std::os::unix::ffi::OsStrExt;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
    let stick = OwnedStick {
        root_path: path.clone(),
        ..owned(1, 10)
    };
    assert!(matches!(
        store.add_owned_stick(&stick),
        Err(StoreError::NonUtf8Path(_))
    ));
    let joined = JoinedStick {
        output_dir: path,
        ..joined(1, 10)
    };
    assert!(matches!(
        store.add_joined_stick(&joined),
        Err(StoreError::NonUtf8Path(_))
    ));
}

#[cfg(not(unix))]
fn non_utf8_paths_are_refused(_store: &dyn Store) {}

fn a_device_made_now_has_no_node_key(store: &dyn Store) {
    let device = Device {
        node_id: None,
        ..device()
    };
    store.create_device(&device).unwrap();
    assert_eq!(store.device().unwrap(), Some(device));
}

fn a_stick_keeps_its_node_id_and_kind(store: &dyn Store) {
    let stick = OwnedStick {
        node_id: Some(NodeId([5; 32])),
        kind: StickKind::App,
        ..owned(1, 10)
    };
    store.add_owned_stick(&stick).unwrap();
    assert_eq!(
        store.owned_stick(&stick.stick_id).unwrap(),
        Some(stick.clone())
    );
    // Two sticks never share a node key.
    let twin = OwnedStick {
        node_id: stick.node_id,
        ..owned(2, 20)
    };
    assert!(matches!(
        store.add_owned_stick(&twin),
        Err(StoreError::StickExists)
    ));
    // Older sticks all have none.
    store.add_owned_stick(&owned(3, 30)).unwrap();
    store.add_owned_stick(&owned(4, 40)).unwrap();
}

macro_rules! for_every_store {
    ($($scenario:ident),* $(,)?) => {
        mod memory {
            $(
                #[test]
                fn $scenario() {
                    super::$scenario(&super::MemoryStore::new());
                }
            )*
        }
        mod sqlite {
            $(
                #[test]
                fn $scenario() {
                    super::$scenario(&super::SqliteStore::open_in_memory().unwrap());
                }
            )*
        }
    };
}

for_every_store!(
    a_device_made_now_has_no_node_key,
    a_stick_keeps_its_node_id_and_kind,
    device_is_created_once,
    device_with_key_file_roundtrips,
    device_with_keychain_roundtrips,
    device_with_credential_manager_roundtrips,
    owned_sticks_roundtrip_in_order,
    a_moved_stick_keeps_everything_but_its_folder,
    a_stick_keeps_its_drive_letter,
    signed_records_only_move_forward,
    signed_record_for_unknown_stick_fails,
    owned_stick_must_be_consistent,
    joined_sticks_roundtrip_in_order,
    values_beyond_i64_are_refused,
    non_utf8_paths_are_refused,
);

// After the macro, so the files scenarios can use it.
mod files;
mod life;
mod linked;
mod received;
mod records;
mod reports;
mod space;
mod waiting;
