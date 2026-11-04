use super::*;
use crate::cipher::CipherId;
use crate::download::files::component_problem;
use crate::stick_id::StickSigningKey;

fn stick(seed: u8, name: &str) -> OwnedStick {
    OwnedStick::new(
        StickSigningKey::from_bytes(&[seed; 32]).stick_id(),
        name,
        format!("/s/{seed}"),
        CipherId::XChaCha20Poly1305,
        1,
    )
}

#[test]
fn a_name_has_one_stored_form() {
    // "ί" as one character and as "ι" with a combining accent.
    let composed = "Μαρία";
    let decomposed = "Μαρι\u{301}α";
    assert_ne!(composed, decomposed);
    assert_eq!(normal_form(decomposed), composed);
    assert_eq!(normal_form("  For Maria \t"), "For Maria");
    assert!(same_name("ΜΑΡΊΑ", decomposed));
    assert!(!same_name("Maria", "Maria 2"));
}

#[test]
fn a_new_name_is_checked_and_stored_in_its_form() {
    let others = [stick(1, "For Maria"), stick(2, "Μαρία")];
    assert_eq!(new_stick_name(" Photos ", &others), Ok("Photos".to_owned()));
    assert_eq!(new_stick_name("   ", &others), Err(NameError::Empty));
    assert_eq!(
        new_stick_name(&"x".repeat(61), &others),
        Err(NameError::TooLong)
    );
    assert_eq!(
        new_stick_name(&"Ω".repeat(60), &others)
            .unwrap()
            .chars()
            .count(),
        60
    );
    assert_eq!(
        new_stick_name("a\u{202e}b", &others),
        Err(NameError::BadCharacter)
    );
    assert_eq!(
        new_stick_name("for maria", &others),
        Err(NameError::Taken("For Maria".to_owned()))
    );
    assert_eq!(
        new_stick_name("Μαρι\u{301}Α", &others),
        Err(NameError::Taken("Μαρία".to_owned()))
    );
}

#[test]
fn a_command_line_name_is_made_fit_never_refused() {
    assert_eq!(name_from_folder("0videos"), "0videos");
    let long = "A Long Film Title (2006) (restored director's cut, with commentaries)";
    assert_eq!(name_from_folder(long).chars().count(), 60);
    assert_eq!(name_from_folder("\u{200b}"), "stick");
}

#[test]
fn a_folder_name_is_one_every_system_takes() {
    let none = |_: &str| false;
    assert_eq!(folder_name("For Maria", none), "For Maria");
    assert_eq!(folder_name("Photos: 2024/25?", none), "Photos_ 2024_25_");
    assert_eq!(folder_name("Notes. ", none), "Notes");
    assert_eq!(folder_name("...", none), "Stick");
    assert_eq!(folder_name("con", none), "con_");
    assert_eq!(folder_name("COM1.txt", none), "COM1_.txt");
    assert_eq!(folder_name("LPT9", none), "LPT9_");
    assert_eq!(folder_name("-rf photos", none), "_rf photos");
    assert_eq!(folder_name("Μαρι\u{301}α", none), "Μαρία");
    // Too long: cut, not given up on.
    let long = folder_name(&"Ταξίδι ".repeat(60), none);
    assert!(long.starts_with("Ταξίδι") && long.encode_utf16().count() <= 255);
    // Taken, as the caller sees it (here ignoring case).
    let taken = |candidate: &str| {
        ["for maria", "for maria (2)"].contains(&candidate.to_lowercase().as_str())
    };
    assert_eq!(folder_name("For Maria", taken), "For Maria (3)");
    for name in ["a<b>", "x\u{7}y", "AUX", "trailing.", "nul.json"] {
        assert!(
            component_problem(&folder_name(name, none), true).is_none(),
            "{name}"
        );
    }
}
