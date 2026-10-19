use super::*;

#[test]
fn a_link_known_here_is_said_with_its_id_and_others_stay_errors() {
    let id = ourstick_core::StickSigningKey::from_bytes(&[4; 32]).stick_id();
    assert!(matches!(known_of(Ok(()), &id), Ok(None)));
    match known_of(Err(ReceivedError::OwnStick("Tutu Miles".to_owned())), &id) {
        Ok(Some(PreviewOutcome::Known {
            known,
            id: hex,
            name,
        })) => {
            assert_eq!((known, name.as_str()), ("own", "Tutu Miles"));
            assert_eq!(hex, crate::state::stick_hex(&id));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        known_of(
            Err(ReceivedError::AlreadyReceived("Family".to_owned())),
            &id
        ),
        Ok(Some(PreviewOutcome::Known {
            known: "received",
            ..
        }))
    ));
    assert!(matches!(
        known_of(Err(ReceivedError::RetiredStick), &id),
        Err(ReceivedError::RetiredStick)
    ));
}

#[test]
fn what_a_choice_needs_leaves_out_each_path_once() {
    let to_write: std::collections::BTreeMap<String, u64> = [
        ("Disk".to_owned(), 10),
        ("Disk/MUSIC".to_owned(), 6),
        ("Disk/Pictures".to_owned(), 3),
        ("note".to_owned(), 1),
    ]
    .into();
    assert_eq!(needs(&to_write, &[]), 11);
    assert_eq!(needs(&to_write, &["Disk/MUSIC".to_owned()]), 5);
    assert_eq!(needs(&to_write, &["Disk".to_owned(), "note".to_owned()]), 0);
    // A path the plan writes nothing of (already there, or gone).
    assert_eq!(needs(&to_write, &["Disk/old".to_owned()]), 11);
}

#[test]
fn a_stick_goes_into_a_folder_named_after_it_never_one_taken() {
    let parent = crate::testing::TempFolder::new();
    let folder = folder_for("Holiday", parent.path());
    assert_eq!(folder, parent.path().join("Holiday"));
    std::fs::create_dir(&folder).unwrap();
    let next = folder_for("Holiday", parent.path());
    assert_ne!(next, folder);
    assert!(next.starts_with(parent.path()));
    // A name with characters no folder may have still gets a folder.
    let odd = folder_for("a/b:c", parent.path());
    assert_eq!(odd.parent().unwrap(), parent.path());
}

#[test]
fn a_preview_and_a_wait_choose_the_same_folder() {
    let parent = crate::testing::TempFolder::new();
    let chosen = parent.path().display().to_string();
    let earlier = parent.path().join("got with the command line");
    // A command-line receive goes on where it is, unless the user chose
    // another place.
    let choose = folder_choice(Some(earlier.clone()), None).unwrap();
    assert_eq!(choose("Holiday"), earlier);
    let choose = folder_choice(Some(earlier), Some(&chosen)).unwrap();
    assert_eq!(choose("Holiday"), parent.path().join("Holiday"));
    let choose = folder_choice(None, Some(&chosen)).unwrap();
    assert_eq!(choose("Holiday"), parent.path().join("Holiday"));
}

#[test]
fn an_unreachable_sender_is_away_but_not_a_stick_it_no_longer_shares() {
    use ourstick_core::download::DownloadError;
    use ourstick_core::transport::TransportError;
    let unreachable = TransportError::Unreachable("gone".to_owned());
    assert!(sender_away(&DownloadError::Transport(unreachable)));
    assert!(!sender_away(&DownloadError::Transport(
        TransportError::UnknownStick
    )));
}

#[test]
fn a_folder_not_made_yet_is_measured_where_it_will_be() {
    let parent = crate::testing::TempFolder::new();
    let deep = parent.path().join("a/b/c");
    assert_eq!(existing(&deep), parent.path());
}
