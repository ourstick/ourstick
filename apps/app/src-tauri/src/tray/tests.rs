use super::*;

fn id(byte: u8) -> StickId {
    ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id()
}

fn thing(shows: Shows, name: &str, words: &str) -> TrayThing {
    TrayThing {
        shows,
        name: name.to_owned(),
        words: words.to_owned(),
    }
}

#[test]
fn each_thing_is_one_line_with_its_sentence_opening_its_view() {
    let photos = thing(
        Shows::Sending(id(1)),
        "Holiday photos",
        "\u{2191} Someone is getting it",
    );
    let videos = thing(
        Shows::Received(id(2)),
        "Family videos",
        "\u{2193} Downloading 45%",
    );
    let lines = menu_lines(&[photos, videos], true);
    assert_eq!(
        lines[2],
        Line::Item(
            "Holiday photos \u{2014} \u{2191} Someone is getting it".to_owned(),
            Action::Show(Shows::Sending(id(1)))
        )
    );
    assert_eq!(
        lines[3],
        Line::Item(
            "Family videos \u{2014} \u{2193} Downloading 45%".to_owned(),
            Action::Show(Shows::Received(id(2)))
        )
    );
    assert_eq!(lines[5], Line::Item("Send…".to_owned(), Action::Send));
    assert_eq!(lines[6], Line::Item("Receive…".to_owned(), Action::Receive));
}

#[test]
fn with_nothing_the_menu_still_opens_sends_receives_and_quits() {
    let lines = menu_lines(&[], false);
    assert_eq!(
        lines[0],
        Line::Item("Open OurStick".to_owned(), Action::Open)
    );
    // Open, a line, Send…, Receive…, a line, About, the check, Quit….
    assert_eq!(lines.len(), 8);
}

#[test]
fn a_native_item_id_names_its_action() {
    for action in [
        Action::Open,
        Action::Show(Shows::Sending(id(3))),
        Action::Show(Shows::Received(id(4))),
        Action::Send,
        Action::Receive,
        Action::About,
        Action::StartAtLogin,
        Action::Quit,
    ] {
        assert_eq!(Action::from_id(&action.id()), Some(action));
    }
    assert_eq!(Action::from_id("sending:zz"), None);
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
                Line::Check(
                    "Open when the computer starts".to_owned(),
                    on,
                    Action::StartAtLogin
                ),
                Line::Item("Quit…".to_owned(), Action::Quit),
            ]
        );
    }
}
