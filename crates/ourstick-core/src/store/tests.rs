//! Behaviour every [`Store`] must have. Each scenario runs against both
//! [`MemoryStore`] and [`SqliteStore`], so the two cannot drift apart.

use super::*;
use crate::record::ManifestRecord;
use crate::stick_id::StickSigningKey;

fn owner(seed: u8) -> StickSigningKey {
    StickSigningKey::from_bytes(&[seed; 32])
}

fn device() -> Device {
    Device {
        node_id: NodeId([9; 32]),
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
        node_id: NodeId([8; 32]),
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
    store.save_signed_record(&first).unwrap();

    let stored = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert_eq!(stored.last_signed_seq, 100);
    assert_eq!(stored.latest_record, Some(first.clone()));

    for seq in [100, 99, 0] {
        assert!(matches!(
            store.save_signed_record(&owner(1).sign_record(seq, [2; 32])),
            Err(StoreError::NotNewer { stored: 100, .. })
        ));
    }
    let unchanged = store.owned_stick(&owner(1).stick_id()).unwrap().unwrap();
    assert_eq!(unchanged.latest_record, Some(first));

    store
        .save_signed_record(&owner(1).sign_record(101, [3; 32]))
        .unwrap();
    assert_eq!(
        store
            .owned_stick(&owner(1).stick_id())
            .unwrap()
            .unwrap()
            .last_signed_seq,
        101
    );
}

fn signed_record_for_unknown_stick_fails(store: &dyn Store) {
    assert!(matches!(
        store.save_signed_record(&owner(1).sign_record(1, [1; 32])),
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

fn floor(store: &dyn Store, seed: u8) -> Option<ManifestVersion> {
    store.record_floor(&owner(seed).stick_id()).unwrap()
}

fn completed(store: &dyn Store, seed: u8) -> Option<ManifestVersion> {
    store
        .joined_stick(&owner(seed).stick_id())
        .unwrap()
        .unwrap()
        .completed
}

fn version(seq: u64, hash: u8) -> ManifestVersion {
    ManifestVersion {
        seq,
        hash: [hash; 32],
    }
}

fn accepted_records_only_move_forward(store: &dyn Store) {
    assert_eq!(floor(store, 1), None);
    store.accept_record(&record(1, 5, 1)).unwrap();
    assert_eq!(floor(store, 1), Some(version(5, 1)));

    for seq in [5, 4, 0] {
        assert!(matches!(
            store.accept_record(&record(1, seq, 2)),
            Err(StoreError::NotNewer { stored: 5, .. })
        ));
    }
    assert_eq!(floor(store, 1), Some(version(5, 1)));
    store.accept_record(&record(1, 6, 3)).unwrap();
    assert_eq!(floor(store, 1), Some(version(6, 3)));
    // Floors are per stick.
    assert_eq!(floor(store, 2), None);
}

fn floor_needs_no_joined_stick_and_leaves_folders_alone(store: &dyn Store) {
    // `ls` raises the floor of a stick that was never downloaded.
    store.accept_record(&record(1, 5, 1)).unwrap();
    assert_eq!(store.joined_stick(&owner(1).stick_id()).unwrap(), None);
    // ... and of one that was: its folder state does not change.
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store
        .complete_download(&owner(1).stick_id(), version(5, 1))
        .unwrap();
    store.accept_record(&record(1, 6, 2)).unwrap();
    assert_eq!(completed(store, 1), Some(version(5, 1)));
}

fn first_record_may_have_seq_zero(store: &dyn Store) {
    store.accept_record(&record(1, 0, 1)).unwrap();
    assert!(matches!(
        store.accept_record(&record(1, 0, 1)),
        Err(StoreError::NotNewer { seq: 0, stored: 0 })
    ));
}

fn completed_download_needs_an_accepted_record(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_joined_stick(&joined(1, 10)).unwrap();
    assert!(matches!(
        store.complete_download(&stick_id, version(1, 1)),
        Err(StoreError::Inconsistent(_))
    ));
    store.accept_record(&record(1, 3, 1)).unwrap();
    assert!(matches!(
        store.complete_download(&stick_id, version(4, 1)),
        Err(StoreError::Inconsistent(_))
    ));
    store.complete_download(&stick_id, version(3, 1)).unwrap();
    assert_eq!(completed(store, 1), Some(version(3, 1)));
    // A stick that was not joined has no folder to complete.
    store.accept_record(&record(2, 1, 1)).unwrap();
    assert!(matches!(
        store.complete_download(&owner(2).stick_id(), version(1, 1)),
        Err(StoreError::StickNotFound)
    ));
}

fn moving_to_another_folder_forgets_what_the_old_one_held(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    assert!(matches!(
        store.move_joined_stick(&stick_id, Path::new("/tmp/elsewhere")),
        Err(StoreError::StickNotFound)
    ));
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store.accept_record(&record(1, 3, 1)).unwrap();
    store.complete_download(&stick_id, version(3, 1)).unwrap();

    store
        .move_joined_stick(&stick_id, Path::new("/tmp/elsewhere"))
        .unwrap();
    let moved = store.joined_stick(&stick_id).unwrap().unwrap();
    assert_eq!(moved.output_dir, PathBuf::from("/tmp/elsewhere"));
    assert_eq!(moved.completed, None);
    assert_eq!(moved.joined_at_ms, 10);
    // The floor stays: it is about the stick, not the folder.
    assert_eq!(floor(store, 1), Some(version(3, 1)));
}

fn values_beyond_i64_are_refused(store: &dyn Store) {
    let too_big = MAX_STORED_SEQ + 1;
    store.add_owned_stick(&owned(1, 10)).unwrap();
    assert!(matches!(
        store.save_signed_record(&owner(1).sign_record(too_big, [1; 32])),
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
        .save_signed_record(&owner(1).sign_record(MAX_STORED_SEQ, [1; 32]))
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
    device_is_created_once,
    device_with_key_file_roundtrips,
    owned_sticks_roundtrip_in_order,
    signed_records_only_move_forward,
    signed_record_for_unknown_stick_fails,
    owned_stick_must_be_consistent,
    joined_sticks_roundtrip_in_order,
    accepted_records_only_move_forward,
    floor_needs_no_joined_stick_and_leaves_folders_alone,
    first_record_may_have_seq_zero,
    completed_download_needs_an_accepted_record,
    moving_to_another_folder_forgets_what_the_old_one_held,
    values_beyond_i64_are_refused,
    non_utf8_paths_are_refused,
);
