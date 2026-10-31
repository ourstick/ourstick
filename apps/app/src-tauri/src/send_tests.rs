use super::*;
use ourstick_core::sender_files::LocalFiles;

/// A home with a main folder and two folders called "Photos".
fn home() -> (crate::testing::TempFolder, PathBuf, SendRules) {
    let dir = crate::testing::TempFolder::new();
    let home = dir.path().join("alice");
    for folder in [
        "Pictures/Holiday 2025/Summer",
        "Pictures/Photos",
        "Expansion/Photos",
        ".ssh",
    ] {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }
    let rules = SendRules {
        home: home.clone(),
        main_folders: vec![home.join("Pictures")],
        ..SendRules::default()
    };
    (dir, home, rules)
}

fn text(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn each_is_checked_with_those_before_it() {
    let (_dir, home, rules) = home();
    let paths = [
        text(home.join("Pictures/Holiday 2025")),
        text(home.join("Pictures/Holiday 2025/Summer")),
        text(home.join(".ssh")),
        text(home.join("Pictures")),
        text(home.join("Pictures/Photos")),
        text(home.join("Expansion/Photos")),
    ];
    let found = verdicts(&LocalFiles, &paths, rules, Vec::new(), None);
    let kinds: Vec<&str> = found.iter().map(|verdict| verdict.kind).collect();
    assert_eq!(
        kinds,
        ["fine", "refuse", "refuse", "refuse", "fine", "refuse"]
    );
    assert_eq!(
        found[1].why.as_deref(),
        Some("\u{201c}Summer\u{201d} is inside \u{201c}Holiday 2025\u{201d}, so it comes along already.")
    );
    assert_eq!(found[5].name, "Photos");
    assert!(found[5]
        .why
        .as_deref()
        .unwrap()
        .starts_with("Two things are called"));
}

#[test]
fn a_main_folder_is_asked_about() {
    let (_dir, home, rules) = home();
    let found = verdicts(
        &LocalFiles,
        &[text(home.join("Pictures"))],
        rules,
        Vec::new(),
        None,
    );
    assert_eq!(found[0].kind, "ask");
    assert_eq!(found[0].ask, Some("main"));
    assert_eq!(
        found[0].question.as_deref(),
        Some("Send all of \u{201c}Pictures\u{201d}?")
    );
}

#[test]
fn what_the_stick_sends_already_takes_its_name() {
    let (_dir, home, rules) = home();
    let taken = vec![("photos".to_owned(), home.join("Expansion/Photos"))];
    let found = verdicts(
        &LocalFiles,
        &[text(home.join("Pictures/Photos"))],
        rules.clone(),
        taken.clone(),
        None,
    );
    assert_eq!(found[0].kind, "refuse");
    assert!(check_all(
        &LocalFiles,
        &[text(home.join("Pictures/Photos"))],
        rules.clone(),
        taken,
        None
    )
    .is_err());
    assert!(check_all(
        &LocalFiles,
        &[text(home.join("Pictures/Photos"))],
        rules,
        Vec::new(),
        None
    )
    .is_ok());
}

#[test]
fn next_to_the_folder_at_the_top_a_name_inside_it_is_refused_at_once() {
    let (_dir, home, rules) = home();
    // The stick was made from "Holiday 2025" alone; it holds "Summer".
    let top = LinkedEntry {
        name: "Holiday 2025".to_owned(),
        path: home.join("Pictures/Holiday 2025"),
        is_dir: true,
        disk: None,
        added_at_ms: 1,
        at_top: true,
    };
    std::fs::create_dir_all(home.join("Expansion/summer")).unwrap();
    std::fs::create_dir_all(home.join("Expansion/Holiday 2025")).unwrap();
    let rules = SendRules {
        chosen: vec![top.path.clone()],
        ..rules
    };
    let found = verdicts(
        &LocalFiles,
        &[
            text(home.join("Expansion/summer")),
            text(home.join("Expansion/Holiday 2025")),
        ],
        rules,
        vec![(top.name.clone(), top.path.clone())],
        Some(&top),
    );
    assert_eq!(found[0].kind, "refuse");
    assert_eq!(
        found[0].why.as_deref(),
        Some("\u{201c}summer\u{201d} is already in \u{201c}Holiday 2025\u{201d}. Rename one first, then add it again.")
    );
    // Its own name stays its own.
    assert_eq!(found[1].kind, "refuse");
    assert!(found[1]
        .why
        .as_deref()
        .unwrap()
        .starts_with("Two things are called"));
}

#[test]
fn a_name_taken_is_said_without_stick() {
    assert_eq!(
        name_words(&NameError::Taken("Holiday".to_owned())),
        "You already send something with this name. Choose another."
    );
}
