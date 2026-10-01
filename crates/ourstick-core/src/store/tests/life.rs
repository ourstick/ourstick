//! A stick's life in the app (ADR 0009 §5), the same in every [`Store`].

use super::*;
use crate::manifest::FileStamp;

fn file(path: &str) -> OwnedFile {
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

fn linked(name: &str) -> LinkedEntry {
    LinkedEntry {
        name: name.to_owned(),
        path: format!("/elsewhere/{name}").into(),
        is_dir: true,
        disk: None,
        added_at_ms: 1,
    }
}

fn claim(stick: &OwnedStick) -> SpaceClaim {
    SpaceClaim {
        stick_id: stick.stick_id,
        kind: ClaimKind::Cache,
        disk: "dev:1".to_owned(),
        bytes: 5,
    }
}

/// A stick with a recorded file, a linked entry and a claim.
fn busy_stick(store: &dyn Store, seed: u8) -> OwnedStick {
    let stick = owned(seed, u64::from(seed) * 10);
    store.add_owned_stick(&stick).unwrap();
    store
        .record_owned_files(&stick.stick_id, &[file("a.txt")])
        .unwrap();
    store
        .add_linked_entry(&stick.stick_id, &linked("Films"))
        .unwrap();
    store.claim_space(&mut |_| Some(claim(&stick))).unwrap();
    stick
}

fn a_stick_being_removed_is_hidden_until_it_is_gone(store: &dyn Store) {
    let kept = busy_stick(store, 1);
    let going = busy_stick(store, 2);
    // Only a stick marked removing is deleted.
    assert!(matches!(
        store.remove_owned_stick(&going.stick_id),
        Err(StoreError::Inconsistent(_))
    ));
    store.mark_removing(&going.stick_id).unwrap();
    assert_eq!(store.owned_stick(&going.stick_id).unwrap(), None);
    assert_eq!(store.owned_sticks().unwrap(), std::slice::from_ref(&kept));
    assert_eq!(
        store.removing_sticks().unwrap(),
        std::slice::from_ref(&going)
    );
    // Nothing changes it any more but its removal.
    assert!(matches!(
        store.rename_owned_stick(&going.stick_id, "x"),
        Err(StoreError::StickNotFound)
    ));
    let signing = owner(2);
    assert!(matches!(
        store.save_signed_record(&signing.sign_record(1, [1; 32]), false),
        Err(StoreError::StickNotFound)
    ));
    assert!(matches!(
        store.record_owned_files(&going.stick_id, &[file("b.txt")]),
        Err(StoreError::StickNotFound)
    ));
    assert!(matches!(
        store.add_linked_entry(&going.stick_id, &linked("Music")),
        Err(StoreError::StickNotFound)
    ));
    assert!(matches!(
        store.mark_removing(&going.stick_id),
        Err(StoreError::StickNotFound)
    ));
    store.remove_owned_stick(&going.stick_id).unwrap();
    assert!(store.removing_sticks().unwrap().is_empty());
    assert!(store.owned_files(&going.stick_id).unwrap().is_empty());
    assert!(store.linked_entries(&going.stick_id).unwrap().is_empty());
    assert_eq!(store.space_claims().unwrap(), [claim(&kept)]);
    // The other stick keeps everything.
    assert_eq!(store.owned_files(&kept.stick_id).unwrap().len(), 1);
    assert_eq!(store.linked_entries(&kept.stick_id).unwrap().len(), 1);
    assert!(matches!(
        store.remove_owned_stick(&going.stick_id),
        Err(StoreError::StickNotFound)
    ));
}

fn a_new_link_takes_the_old_ones_place_and_retires_it(store: &dyn Store) {
    let old = busy_stick(store, 1);
    let new = OwnedStick {
        root_path: old.root_path.clone(),
        name: old.name.clone(),
        node_id: Some(crate::ticket::NodeId([42; 32])),
        ..owned(2, 20)
    };
    store.replace_owned_stick(&old.stick_id, &new, 99).unwrap();
    assert_eq!(store.owned_sticks().unwrap(), std::slice::from_ref(&new));
    assert_eq!(store.owned_stick(&old.stick_id).unwrap(), None);
    assert!(store.removing_sticks().unwrap().is_empty());
    // Its linked entries follow; the old catalogue and claim do not.
    assert_eq!(
        store.linked_entries(&new.stick_id).unwrap(),
        [linked("Films")]
    );
    assert!(store.linked_entries(&old.stick_id).unwrap().is_empty());
    assert!(store.owned_files(&old.stick_id).unwrap().is_empty());
    assert!(store.owned_files(&new.stick_id).unwrap().is_empty());
    assert!(store.space_claims().unwrap().is_empty());
    // The old id waits for its secrets and cache to go.
    assert_eq!(
        store.retired_sticks().unwrap(),
        [RetiredStick {
            old: old.stick_id,
            replaced_by: new.stick_id
        }]
    );
    store.forget_retired(&old.stick_id).unwrap();
    assert!(store.retired_sticks().unwrap().is_empty());

    // Refused: the same id, a gone stick, a stick being removed.
    assert!(matches!(
        store.replace_owned_stick(&new.stick_id, &new, 1),
        Err(StoreError::Inconsistent(_))
    ));
    assert!(matches!(
        store.replace_owned_stick(&old.stick_id, &owned(3, 30), 1),
        Err(StoreError::StickNotFound)
    ));
    store.mark_removing(&new.stick_id).unwrap();
    assert!(matches!(
        store.replace_owned_stick(&new.stick_id, &owned(4, 40), 1),
        Err(StoreError::StickNotFound)
    ));
}

fn a_stick_keeps_its_name_wish_and_image(store: &dyn Store) {
    let stick = owned(1, 10);
    store.add_owned_stick(&stick).unwrap();
    assert!(stick.sharing_wanted);
    store
        .rename_owned_stick(&stick.stick_id, "For Maria")
        .unwrap();
    store.set_sharing_wanted(&stick.stick_id, false).unwrap();
    store
        .set_image_path(&stick.stick_id, Path::new("/Users/a/OurStick.sparseimage"))
        .unwrap();
    let read = store.owned_stick(&stick.stick_id).unwrap().unwrap();
    assert_eq!(read.name, "For Maria");
    assert!(!read.sharing_wanted);
    assert_eq!(
        read.image_path.as_deref(),
        Some(Path::new("/Users/a/OurStick.sparseimage"))
    );
    let missing = owner(9).stick_id();
    assert!(matches!(
        store.set_sharing_wanted(&missing, true),
        Err(StoreError::StickNotFound)
    ));
}

for_every_store!(
    a_stick_being_removed_is_hidden_until_it_is_gone,
    a_new_link_takes_the_old_ones_place_and_retires_it,
    a_stick_keeps_its_name_wish_and_image,
);
