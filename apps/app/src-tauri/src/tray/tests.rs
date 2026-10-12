use super::*;

fn stick(byte: u8, name: &str, mounted: bool, sharing: bool) -> TrayStick {
    TrayStick {
        id: ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id(),
        name: name.to_owned(),
        mounted,
        sharing,
        doing: None,
    }
}

#[test]
fn the_toggling_items_say_what_they_will_do() {
    assert_eq!(mount_label(true), "Eject");
    assert_eq!(mount_label(false), "Mount");
    assert_eq!(sharing_label(true), "Stop sharing…");
    assert_eq!(sharing_label(false), "Start sharing");
}

#[test]
fn each_stick_has_its_own_submenu() {
    let photos = stick(1, "Photos", true, true);
    let films = stick(2, "Films", false, false);
    let lines = menu_lines(&[photos.clone(), films.clone()], true);
    assert_eq!(lines.len(), 10);
    assert_eq!(
        lines[2],
        Line::Stick(
            "Photos \u{2014} sharing".to_owned(),
            vec![
                ("Share…".to_owned(), Action::Share(photos.id)),
                ("Eject".to_owned(), Action::Mount(photos.id)),
                ("Stop sharing…".to_owned(), Action::StopSharing(photos.id)),
            ]
        )
    );
    let Line::Stick(label, items) = &lines[3] else {
        panic!("{:?}", lines[3]);
    };
    assert_eq!(label, "Films \u{2014} not sharing");
    assert_eq!(items[1].0, "Mount");
    assert_eq!(
        items[2],
        ("Start sharing".to_owned(), Action::StartSharing(films.id))
    );
    assert_eq!(
        lines[5],
        Line::Item("New stick…".to_owned(), Action::NewStick)
    );
}

#[test]
fn a_busy_stick_says_what_it_is_doing_and_offers_nothing() {
    let renaming = TrayStick {
        doing: Some("Renaming…"),
        ..stick(4, "Photos", true, true)
    };
    let lines = menu_lines(&[renaming], true);
    assert_eq!(lines[2], Line::Note("Photos \u{2014} Renaming…".to_owned()));
}

#[test]
fn with_no_stick_the_menu_still_opens_makes_and_quits() {
    let lines = menu_lines(&[], false);
    assert_eq!(
        lines[0],
        Line::Item("Open OurStick".to_owned(), Action::Open)
    );
    // Open, a line, New stick…, a line, About, the check, Quit.
    assert_eq!(lines.len(), 7);
}

#[test]
fn a_native_item_id_names_its_action() {
    let id = stick(3, "Photos", true, true).id;
    for action in [
        Action::Open,
        Action::Share(id),
        Action::Mount(id),
        Action::StopSharing(id),
        Action::StartSharing(id),
        Action::NewStick,
        Action::Quit,
    ] {
        assert_eq!(Action::from_id(&action.id()), Some(action));
    }
    assert_eq!(Action::from_id("share:zz"), None);
    assert_eq!(Action::from_id("eject"), None);
}

#[test]
fn about_and_the_login_check_come_before_quit() {
    for on in [true, false] {
        let lines = menu_lines(&[], on);
        let end = &lines[lines.len() - 3..];
        assert_eq!(
            end,
            [
                Line::Item("About OurStick".to_owned(), Action::About),
                Line::Check("Start at login".to_owned(), on, Action::StartAtLogin),
                Line::Item("Quit".to_owned(), Action::Quit),
            ]
        );
    }
}
