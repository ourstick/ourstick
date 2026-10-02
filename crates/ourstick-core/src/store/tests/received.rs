//! Received sticks in the receiver window (step 7b): the same in every
//! [`Store`].

use super::*;
use crate::manifest::FileStamp;
use crate::store::{ClaimKind, JoinedChange, SpaceClaim, StickKind};

fn a_received_stick_keeps_its_program_name_and_pause(store: &dyn Store) {
    let stick = JoinedStick {
        kind: StickKind::App,
        name: Some("Holiday".to_owned()),
        ..joined(1, 10)
    };
    store.add_joined_stick(&stick).unwrap();
    assert_eq!(
        store.joined_stick(&stick.stick_id).unwrap(),
        Some(stick.clone())
    );
    let id = stick.stick_id;
    store
        .change_joined_stick(&id, &JoinedChange::Name("Holiday 2026".to_owned()))
        .unwrap();
    store
        .change_joined_stick(&id, &JoinedChange::Paused(true))
        .unwrap();
    store
        .change_joined_stick(&id, &JoinedChange::Kind(StickKind::Cli))
        .unwrap();
    let read = store.joined_stick(&id).unwrap().unwrap();
    assert_eq!(read.name.as_deref(), Some("Holiday 2026"));
    assert!(read.paused);
    assert_eq!(read.kind, StickKind::Cli);
    let missing = owner(9).stick_id();
    assert!(matches!(
        store.change_joined_stick(&missing, &JoinedChange::Paused(true)),
        Err(StoreError::StickNotFound)
    ));
}

fn a_stick_being_forgotten_is_hidden_and_goes_but_its_floor_stays(store: &dyn Store) {
    let stick = joined(2, 10);
    let id = stick.stick_id;
    store.add_joined_stick(&stick).unwrap();
    store.add_joined_stick(&joined(3, 11)).unwrap();
    store.accept_record(&record(2, 5, 1)).unwrap();
    let file = DownloadedFile {
        path: "a.jpg".to_owned(),
        saved_as: "a.jpg".to_owned(),
        stamp: FileStamp {
            size: 1,
            mtime_ns: 2,
            ctime_ns: 3,
        },
        blob_hash: [7; 32],
    };
    store.record_downloaded_file(&id, &file).unwrap();
    store.set_left_out(&id, &["Films".to_owned()]).unwrap();
    let claim = SpaceClaim {
        stick_id: id,
        kind: ClaimKind::Download,
        disk: "dev:1".to_owned(),
        bytes: 5,
    };
    store.claim_space(&mut |_| vec![claim.clone()]).unwrap();
    // Not forgotten unless marked first.
    assert!(store.forget_joined_stick(&id).is_err());
    store
        .change_joined_stick(&id, &JoinedChange::Forgetting)
        .unwrap();
    assert!(store.joined_stick(&id).unwrap().is_none());
    assert_eq!(store.joined_sticks().unwrap().len(), 1);
    assert_eq!(store.forgetting_joined_sticks().unwrap()[0].stick_id, id);
    // Nothing changes it meanwhile.
    assert!(store
        .change_joined_stick(&id, &JoinedChange::Paused(true))
        .is_err());
    assert!(store
        .move_joined_stick(&id, Path::new("/elsewhere"))
        .is_err());
    assert!(store.record_downloaded_file(&id, &file).is_err());
    assert!(store.set_left_out(&id, &[]).is_err());
    store.forget_joined_stick(&id).unwrap();
    assert!(store.left_out(&id).unwrap().is_empty());
    assert!(store.forgetting_joined_sticks().unwrap().is_empty());
    assert!(store.downloaded_files(&id).unwrap().is_empty());
    assert!(store.space_claims().unwrap().is_empty());
    assert_eq!(store.record_floor(&id).unwrap().unwrap().seq, 5);
    // Added again from its link: a new received stick.
    store.add_joined_stick(&stick).unwrap();
}

fn what_is_left_out_is_kept_whole_and_replaced_whole(store: &dyn Store) {
    let id = joined(4, 10).stick_id;
    // No such stick: refused.
    assert!(matches!(
        store.set_left_out(&id, &["Films".to_owned()]),
        Err(StoreError::StickNotFound)
    ));
    store.add_joined_stick(&joined(4, 10)).unwrap();
    // Nothing left out: everything is got.
    assert!(store.left_out(&id).unwrap().is_empty());
    let names =
        |list: &[&str]| -> Vec<String> { list.iter().map(|name| (*name).to_owned()).collect() };
    // Byte-exact (a Mac's decomposed name is another name), sorted, once.
    let decomposed = "Φω\u{301}τα";
    store
        .set_left_out(&id, &names(&["Films", decomposed, "Films", "Old photos"]))
        .unwrap();
    assert_eq!(
        store.left_out(&id).unwrap(),
        names(&["Films", "Old photos", decomposed])
    );
    // Replaced whole.
    store.set_left_out(&id, &names(&["Music"])).unwrap();
    assert_eq!(store.left_out(&id).unwrap(), names(&["Music"]));
    store.set_left_out(&id, &[]).unwrap();
    assert!(store.left_out(&id).unwrap().is_empty());
    // An empty name is not a name.
    assert!(store.set_left_out(&id, &names(&[""])).is_err());
    assert!(store.left_out(&id).unwrap().is_empty());
    // Per stick.
    store.add_joined_stick(&joined(5, 11)).unwrap();
    store.set_left_out(&id, &names(&["Films"])).unwrap();
    assert!(store.left_out(&joined(5, 11).stick_id).unwrap().is_empty());
}

for_every_store!(
    what_is_left_out_is_kept_whole_and_replaced_whole,
    a_received_stick_keeps_its_program_name_and_pause,
    a_stick_being_forgotten_is_hidden_and_goes_but_its_floor_stays,
);
