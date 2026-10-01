//! Linked entries (ADR 0008) every [`Store`] must keep the same way.

use super::*;
use crate::manifest::FileStamp;

fn linked(name: &str, path: &str) -> LinkedEntry {
    LinkedEntry {
        name: name.to_owned(),
        path: path.into(),
        is_dir: true,
        disk: Some(u64::MAX - 1),
        added_at_ms: 7,
    }
}

fn owned_file(path: &str) -> OwnedFile {
    OwnedFile {
        path: path.to_owned(),
        stamp: FileStamp {
            size: 1,
            mtime_ns: 2,
            ctime_ns: 3,
        },
        blob_hash: [4; 32],
        plaintext_hash: None,
    }
}

fn linked_entries_roundtrip_by_name(store: &dyn Store) {
    let stick = owned(1, 10);
    store.add_owned_stick(&stick).unwrap();
    let films = linked("Films", "/media/disk/Films");
    let notes = LinkedEntry {
        is_dir: false,
        disk: None,
        ..linked("notes.txt", "/home/me/notes.txt")
    };
    store.add_linked_entry(&stick.stick_id, &notes).unwrap();
    store.add_linked_entry(&stick.stick_id, &films).unwrap();
    assert_eq!(
        store.linked_entries(&stick.stick_id).unwrap(),
        vec![films.clone(), notes]
    );
    // Per stick.
    let other = owned(2, 20);
    store.add_owned_stick(&other).unwrap();
    assert_eq!(store.linked_entries(&other.stick_id).unwrap(), vec![]);
    // One name once.
    assert!(matches!(
        store.add_linked_entry(&stick.stick_id, &linked("Films", "/elsewhere")),
        Err(StoreError::LinkExists(name)) if name == "Films"
    ));
    // Only for a stick this device owns.
    assert!(matches!(
        store.add_linked_entry(&owner(9).stick_id(), &films),
        Err(StoreError::StickNotFound)
    ));
}

fn a_linked_entry_must_be_one_name_and_an_absolute_path(store: &dyn Store) {
    let stick = owned(1, 10);
    store.add_owned_stick(&stick).unwrap();
    for bad in [
        linked("", "/a"),
        linked("..", "/a"),
        linked("a/b", "/a"),
        linked("a\\b", "/a"),
        linked("a", "relative/path"),
    ] {
        assert!(
            matches!(
                store.add_linked_entry(&stick.stick_id, &bad),
                Err(StoreError::Inconsistent(_))
            ),
            "{bad:?}"
        );
    }
}

fn removing_a_linked_entry_forgets_only_its_files(store: &dyn Store) {
    let stick = owned(1, 10);
    store.add_owned_stick(&stick).unwrap();
    store
        .add_linked_entry(&stick.stick_id, &linked("Films", "/media/disk/Films"))
        .unwrap();
    store
        .record_owned_files(
            &stick.stick_id,
            &[
                owned_file("Films/a.mkv"),
                owned_file("Films/b/c.mkv"),
                owned_file("Films 2/d.mkv"),
                owned_file("Filmsx"),
                owned_file("own.txt"),
            ],
        )
        .unwrap();
    store.remove_linked_entry(&stick.stick_id, "Films").unwrap();
    assert_eq!(store.linked_entries(&stick.stick_id).unwrap(), vec![]);
    let left: Vec<String> = store
        .owned_files(&stick.stick_id)
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(left, ["Films 2/d.mkv", "Filmsx", "own.txt"]);
    assert!(matches!(
        store.remove_linked_entry(&stick.stick_id, "Films"),
        Err(StoreError::LinkNotFound(_))
    ));
}

for_every_store!(
    linked_entries_roundtrip_by_name,
    a_linked_entry_must_be_one_name_and_an_absolute_path,
    removing_a_linked_entry_forgets_only_its_files,
);
