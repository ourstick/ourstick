//! Records and what folders hold, which every [`Store`] must keep the same way.

use super::*;

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

for_every_store!(
    accepted_records_only_move_forward,
    floor_needs_no_joined_stick_and_leaves_folders_alone,
    first_record_may_have_seq_zero,
    completed_download_needs_an_accepted_record,
    moving_to_another_folder_forgets_what_the_old_one_held,
);
