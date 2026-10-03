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
        plaintext_hash: Some([hash ^ 0xff; 32]),
    }
}

fn owned_files_are_recorded_in_parts_and_pruned_at_the_end(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_owned_stick(&owned(1, 10)).unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![]);

    store
        .record_owned_files(&stick_id, &[file("b", 2, 2), file("a/x", 1, 1)])
        .unwrap();
    let mut old = file("c", 4, 4);
    old.plaintext_hash = None;
    store.record_owned_files(&stick_id, &[old.clone()]).unwrap();
    assert_eq!(
        store.owned_files(&stick_id).unwrap(),
        vec![file("a/x", 1, 1), file("b", 2, 2), old]
    );
    // A later part replaces the row of the same path.
    store
        .record_owned_files(&stick_id, &[file("b", 3, 3)])
        .unwrap();
    // A file that left the folder leaves the record at the end of the pass.
    store
        .keep_only_owned_files(&stick_id, &["b".to_owned(), "gone".to_owned()])
        .unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![file("b", 3, 3)]);
    store
        .record_owned_files(&stick_id, &[file("c", 5, 5)])
        .unwrap();
    store
        .forget_owned_files(&stick_id, &["b".to_owned(), "never".to_owned()])
        .unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![file("c", 5, 5)]);
    store.keep_only_owned_files(&stick_id, &[]).unwrap();
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![]);
}

fn owned_files_need_an_owned_stick(store: &dyn Store) {
    assert!(matches!(
        store.record_owned_files(&owner(1).stick_id(), &[file("a", 1, 1)]),
        Err(StoreError::StickNotFound)
    ));
    assert!(matches!(
        store.keep_only_owned_files(&owner(1).stick_id(), &[]),
        Err(StoreError::StickNotFound)
    ));
    assert!(matches!(
        store.forget_owned_files(&owner(1).stick_id(), &[]),
        Err(StoreError::StickNotFound)
    ));
}

fn owned_files_are_per_stick(store: &dyn Store) {
    store.add_owned_stick(&owned(1, 10)).unwrap();
    store.add_owned_stick(&owned(2, 20)).unwrap();
    store
        .record_owned_files(&owner(1).stick_id(), &[file("a", 1, 1)])
        .unwrap();
    store
        .record_owned_files(&owner(2).stick_id(), &[file("a", 2, 2)])
        .unwrap();
    store
        .keep_only_owned_files(&owner(2).stick_id(), &[])
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
        .record_owned_files(&stick_id, &[file("a", 1, 1)])
        .unwrap();
    let mut huge = file("b", 1, 1);
    huge.stamp.mtime_ns = u64::MAX;
    assert!(matches!(
        store.record_owned_files(&stick_id, &[file("c", 1, 1), huge]),
        Err(StoreError::ValueOutOfRange("mtime", _))
    ));
    // Nothing of that part was recorded.
    assert_eq!(store.owned_files(&stick_id).unwrap(), vec![file("a", 1, 1)]);
}

fn downloaded(path: &str, hash: u8) -> DownloadedFile {
    DownloadedFile {
        path: path.to_owned(),
        saved_as: path.to_owned(),
        stamp: file(path, 1, hash).stamp,
        blob_hash: [hash; 32],
    }
}

fn downloaded_files_are_recorded_one_by_one(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    assert!(matches!(
        store.record_downloaded_file(&stick_id, &downloaded("a", 1)),
        Err(StoreError::StickNotFound)
    ));
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store
        .record_downloaded_file(&stick_id, &downloaded("b", 2))
        .unwrap();
    store
        .record_downloaded_file(&stick_id, &downloaded("a", 1))
        .unwrap();
    // A newer version of the same file replaces its row.
    store
        .record_downloaded_file(&stick_id, &downloaded("b", 3))
        .unwrap();
    assert_eq!(
        store.downloaded_files(&stick_id).unwrap(),
        vec![downloaded("a", 1), downloaded("b", 3)]
    );
    assert_eq!(
        store.downloaded_files(&owner(2).stick_id()).unwrap(),
        vec![]
    );
}

fn moving_the_folder_forgets_the_downloaded_files(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store
        .record_downloaded_file(&stick_id, &downloaded("a", 1))
        .unwrap();
    store
        .move_joined_stick(&stick_id, std::path::Path::new("/tmp/elsewhere"))
        .unwrap();
    assert_eq!(store.downloaded_files(&stick_id).unwrap(), vec![]);
}

fn a_file_saved_under_another_name_is_recorded_so(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_joined_stick(&joined(1, 10)).unwrap();
    let renamed = DownloadedFile {
        saved_as: "a_.jpg".to_owned(),
        ..downloaded("a?.jpg", 1)
    };
    store.record_downloaded_file(&stick_id, &renamed).unwrap();
    store
        .record_downloaded_file(&stick_id, &downloaded("b", 2))
        .unwrap();
    assert_eq!(
        store.downloaded_files(&stick_id).unwrap(),
        vec![renamed.clone(), downloaded("b", 2)]
    );
    assert_eq!(
        store.saved_elsewhere(&stick_id).unwrap(),
        vec![renamed.clone()]
    );
    // A later version of it records its place again (here, a new one).
    let moved = DownloadedFile {
        saved_as: "a_ (2).jpg".to_owned(),
        ..renamed.clone()
    };
    store.record_downloaded_file(&stick_id, &moved).unwrap();
    assert_eq!(store.saved_elsewhere(&stick_id).unwrap(), vec![moved]);
}

fn two_files_are_never_recorded_at_one_place(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store
        .record_downloaded_file(&stick_id, &downloaded("a_.jpg", 1))
        .unwrap();
    let onto = DownloadedFile {
        saved_as: "a_.jpg".to_owned(),
        ..downloaded("a?.jpg", 2)
    };
    assert!(matches!(
        store.record_downloaded_file(&stick_id, &onto),
        Err(StoreError::Inconsistent(_))
    ));
    assert_eq!(
        store.downloaded_files(&stick_id).unwrap(),
        vec![downloaded("a_.jpg", 1)]
    );
    // Once the other is forgotten, the place is free.
    store.forget_downloaded_file(&stick_id, "a_.jpg").unwrap();
    store.record_downloaded_file(&stick_id, &onto).unwrap();
    // Forgetting what is not recorded does nothing.
    store.forget_downloaded_file(&stick_id, "never").unwrap();
    store
        .forget_downloaded_file(&owner(5).stick_id(), "a?.jpg")
        .unwrap();
    assert_eq!(store.downloaded_files(&stick_id).unwrap(), vec![onto]);
}

fn downloaded_files_are_recorded_in_groups_whole_or_not_at_all(store: &dyn Store) {
    let stick_id = owner(1).stick_id();
    let group = [downloaded("a", 1), downloaded("b", 2)];
    assert!(matches!(
        store.record_downloaded_files(&stick_id, &group),
        Err(StoreError::StickNotFound)
    ));
    store.add_joined_stick(&joined(1, 10)).unwrap();
    store.record_downloaded_files(&stick_id, &[]).unwrap();
    store.record_downloaded_files(&stick_id, &group).unwrap();
    assert_eq!(store.downloaded_files(&stick_id).unwrap(), group);
    // A row at another file's place, in the group or already there: none of
    // the group is kept, also the rows before it.
    let onto_b = DownloadedFile {
        saved_as: "b".to_owned(),
        ..downloaded("c", 3)
    };
    for bad in [
        vec![downloaded("d", 4), onto_b.clone()],
        vec![
            downloaded("d", 4),
            DownloadedFile {
                saved_as: "d".to_owned(),
                ..downloaded("e", 5)
            },
        ],
    ] {
        assert!(matches!(
            store.record_downloaded_files(&stick_id, &bad),
            Err(StoreError::Inconsistent(_))
        ));
        assert_eq!(store.downloaded_files(&stick_id).unwrap(), group);
    }
    // A bad stamp: refused before anything is written.
    let mut huge = downloaded("f", 6);
    huge.stamp.size = u64::MAX;
    assert!(store
        .record_downloaded_files(&stick_id, &[downloaded("g", 7), huge])
        .is_err());
    assert_eq!(store.downloaded_files(&stick_id).unwrap(), group);
    // In order: a later row of the group replaces an earlier one, and a row
    // may take a place an earlier row of the group gave up.
    let b_moves = DownloadedFile {
        saved_as: "b2".to_owned(),
        ..downloaded("b", 8)
    };
    store
        .record_downloaded_files(&stick_id, &[b_moves.clone(), onto_b.clone()])
        .unwrap();
    assert_eq!(
        store.downloaded_files(&stick_id).unwrap(),
        vec![downloaded("a", 1), b_moves, onto_b]
    );
    // A stick being forgotten records nothing.
    store
        .change_joined_stick(&stick_id, &JoinedChange::Forgetting)
        .unwrap();
    assert!(matches!(
        store.record_downloaded_files(&stick_id, &[downloaded("h", 9)]),
        Err(StoreError::StickNotFound)
    ));
}

for_every_store!(
    downloaded_files_are_recorded_in_groups_whole_or_not_at_all,
    a_file_saved_under_another_name_is_recorded_so,
    two_files_are_never_recorded_at_one_place,
    downloaded_files_are_recorded_one_by_one,
    moving_the_folder_forgets_the_downloaded_files,
    owned_files_are_recorded_in_parts_and_pruned_at_the_end,
    owned_files_need_an_owned_stick,
    owned_files_are_per_stick,
    owned_file_values_beyond_i64_are_refused,
);
