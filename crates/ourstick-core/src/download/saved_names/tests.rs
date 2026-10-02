use super::*;

const WINDOWS: bool = true;
const ELSEWHERE: bool = false;

#[test]
fn a_name_that_can_be_saved_is_left_as_it_is() {
    for name in [
        "photo.jpg",
        "Μαρία.txt",
        "😀 party",
        "שלום.doc",
        ".profile",
        "a b",
    ] {
        assert!(
            matches!(safe_name(name, WINDOWS), Cow::Borrowed(_)),
            "{name}"
        );
        assert!(
            matches!(safe_name(name, ELSEWHERE), Cow::Borrowed(_)),
            "{name}"
        );
    }
}

#[test]
fn windows_characters_become_underscores() {
    assert_eq!(safe_name("What a view?.jpg", WINDOWS), "What a view_.jpg");
    assert_eq!(safe_name("???.txt", WINDOWS), "___.txt");
    assert_eq!(safe_name("a<b>c:d\"e|f*g", WINDOWS), "a_b_c_d_e_f_g");
    assert_eq!(safe_name("tab\there", WINDOWS), "tab_here");
    // Allowed elsewhere: left as they are.
    assert_eq!(safe_name("What a view?.jpg", ELSEWHERE), "What a view?.jpg");
}

#[test]
fn windows_ends_and_reserved_names() {
    assert_eq!(safe_name("Notes.", WINDOWS), "Notes");
    assert_eq!(safe_name("Notes . ", WINDOWS), "Notes");
    assert_eq!(safe_name("...", WINDOWS), "_");
    assert_eq!(safe_name("CON.txt", WINDOWS), "CON_.txt");
    assert_eq!(safe_name("com1", WINDOWS), "com1_");
    assert_eq!(safe_name("LPT9.tar.gz", WINDOWS), "LPT9_.tar.gz");
    // Elsewhere these are plain names.
    assert_eq!(safe_name("Notes.", ELSEWHERE), "Notes.");
    assert_eq!(safe_name("CON.txt", ELSEWHERE), "CON.txt");
}

#[test]
fn a_drive_is_refused_everywhere() {
    assert_eq!(safe_name("c:notes", ELSEWHERE), "c_notes");
    assert_eq!(safe_name("c:notes", WINDOWS), "c_notes");
    // A colon elsewhere in the name is only Windows' business.
    assert_eq!(safe_name("ab:c", ELSEWHERE), "ab:c");
}

#[test]
fn a_long_name_is_cut_keeping_its_extension() {
    let long = format!("{}.jpg", "a".repeat(300));
    let cut = safe_name(&long, ELSEWHERE);
    assert!(cut.ends_with(".jpg"));
    assert_eq!(cut.len(), MAX_NAME - NUMBER_ROOM);
    // Greek letters take two bytes each but one UTF-16 unit.
    let greek = format!("{}.txt", "φ".repeat(200));
    assert!(matches!(safe_name(&greek, WINDOWS), Cow::Borrowed(_)));
    let cut = safe_name(&greek, ELSEWHERE);
    assert!(cut.ends_with(".txt") && cut.len() <= MAX_NAME);
    assert!(cut.is_char_boundary(cut.len() - 4));
    // An extension that takes half the room is cut with the rest.
    let odd = format!("a.{}", "x".repeat(300));
    assert!(safe_name(&odd, ELSEWHERE).len() <= MAX_NAME);
}

#[test]
fn a_cut_does_not_split_a_joined_emoji() {
    // 👩‍👩‍👧: three emoji joined by U+200D.
    let family = "👩\u{200d}👩\u{200d}👧";
    let long = family.repeat(40);
    let cut = safe_name(&long, ELSEWHERE);
    assert!(!cut.ends_with('\u{200d}'), "{cut:?}");
}

#[test]
fn a_cut_name_follows_the_end_rules_again() {
    // Cut where a dot now ends it.
    let name = format!(
        "{}.{}",
        "a".repeat(MAX_NAME - NUMBER_ROOM - 1),
        "b".repeat(200)
    );
    let saved = safe_name(&name, WINDOWS);
    assert!(!saved.ends_with('.'), "{saved}");
    assert!(component_problem(&saved, WINDOWS).is_none());
}

#[test]
fn numbered_names_fit() {
    assert_eq!(numbered("photo.jpg", 2, WINDOWS), "photo (2).jpg");
    assert_eq!(numbered("Trip", 3, ELSEWHERE), "Trip (3)");
    assert_eq!(numbered(".profile", 2, ELSEWHERE), ".profile (2)");
    let long = format!("{}.jpg", "a".repeat(250));
    let numbered = numbered(&long, 12, ELSEWHERE);
    assert!(numbered.ends_with(" (12).jpg") && numbered.len() <= MAX_NAME);
}

#[test]
fn ourstick_names_get_a_mark_in_front() {
    let ours = [".ourstick-partial", ".ourstick-received"];
    assert_eq!(
        not_ours(".OurStick-Partial", &ours).as_deref(),
        Some("_.OurStick-Partial")
    );
    assert_eq!(not_ours("photos", &ours), None);
}

#[test]
fn any_name_becomes_savable_and_stays_so() {
    let names = [
        "a?b",
        "CON",
        "con.txt",
        "...",
        " ",
        "c:",
        "x:",
        "a.",
        "a. .",
        "*",
        "\u{1}",
        "\u{7f}name",
        "COM¹.txt",
        "😀?😀",
        "Μαρία:",
        "\u{202e}abc",
    ];
    let mut all: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    all.push("x".repeat(400));
    all.push(format!("{}?", "é".repeat(200)));
    all.push(format!("CON.{}", "x".repeat(300)));
    for name in &all {
        for windows in [WINDOWS, ELSEWHERE] {
            let saved = safe_name(name, windows);
            assert!(
                component_problem(&saved, windows).is_none(),
                "{name:?} → {saved:?} (windows: {windows})"
            );
            assert_eq!(
                safe_name(&saved, windows),
                saved,
                "not idempotent: {name:?}"
            );
            let again = numbered(&saved, 7, windows);
            assert!(component_problem(&again, windows).is_none(), "{again:?}");
        }
    }
}
