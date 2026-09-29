//! Per-file state every [`Store`] must keep the same way.

use super::*;
use crate::manifest::FileStamp;

fn file(path: &str, size: u64, hash: u8) -> OwnedFile {
    OwnedFile {
        path: path.to_owned(),
        stamp: FileStamp {
            size,
            mtime_ns: 1_700_000_000_123_456_789,
            ctime_ns: 1_700_000_000_987_654_321,
        },
        blob_hash: [hash; 32],
    }
}

fn owned_files_are_replaced_as_a_whole(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_owned_stick(&owned(1, 10)).unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![]);

    store
        .replace_owned_files(&stick_id, &[file("b", 2, 2), file("a/x", 1, 1)])
        .unwrap();
    assert_eq!(
        store.owned_files(&stick_id).unwrap(),
        vec![file("a/x", 1, 1), file("b", 2, 2)]
    );
    // A file that left the folder leaves the record too.
    store
        .replace_owned_files(&stick_id, &[file("b", 3, 3)])
        .unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![file("b", 3, 3)]);
    store.replace_owned_files(&stick_id, &[]).unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![]);
}

fn owned_files_need_an_owned_stick(store: &dyn Store) {
    assert!(matches!(
        store.replace_owned_files(&owner(1).stick_id(), &[file("a", 1, 1)]),
        Err(StoreError::StickNotFound)
    ));
}

fn owned_files_are_per_stick(store: &dyn Store) {
    store.add_owned_stick(&owned(1, 10)).unwrap();
    store.add_owned_stick(&owned(2, 20)).unwrap();
    store
        .replace_owned_files(&owner(1).stick_id(), &[file("a", 1, 1)])
        .unwrap();
    store
        .replace_owned_files(&owner(2).stick_id(), &[file("a", 2, 2)])
        .unwrap();
    assert_eq!(
        store.owned_files(&owner(1).stick_id()).unwrap(),
        vec![file("a", 1, 1)]
    );
}

fn owned_file_values_beyond_i64_are_refused(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_owned_stick(&owned(1, 10)).unwrap();
    store
        .replace_owned_files(&stick_id, &[file("a", 1, 1)])
        .unwrap();
    let mut huge = file("b", 1, 1);
    huge.stamp.mtime_ns = u64::MAX;
    assert!(matches!(
        store.replace_owned_files(&stick_id, &[file("c", 1, 1), huge]),
        Err(StoreError::ValueOutOfRange("mtime", _))
    ));
    // Nothing was replaced.
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![file("a", 1, 1)]);
}

for_every_store!(
    owned_files_are_replaced_as_a_whole,
    owned_files_need_an_owned_stick,
    owned_files_are_per_stick,
    owned_file_values_beyond_i64_are_refused,
);
