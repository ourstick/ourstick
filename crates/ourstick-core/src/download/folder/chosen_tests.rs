//! Plans with what the user chose not to get (ADR 0012).

use super::tests::{cipher, entry, manifest};
use super::*;

#[test]
fn what_is_not_chosen_is_not_fetched_nor_counted() {
    let out = tempfile::tempdir().unwrap();
    let entries = || {
        vec![
            entry("Films/a.mkv", b"aaaa"),
            entry("Films/b.mkv", b"bb"),
            entry("Music/c.mp3", b"c"),
            entry("note.txt", b"n"),
        ]
    };
    let plan = plan_folder(
        out.path(),
        &manifest(entries()),
        &[],
        &["Films".to_owned(), "Gone".to_owned()],
        &cipher(),
    )
    .unwrap();
    assert_eq!(&plan.files[..2], [FilePlan::NotChosen, FilePlan::NotChosen]);
    assert_eq!(plan.bytes_to_write(&manifest(entries())), 2);
    // Told by top-level name, with what it holds; a name not on the stick
    // is not told.
    assert_eq!(plan.not_chosen, [("Films".to_owned(), 2, 6)]);
    assert!(plan.renamed.is_empty());
    // A preview's per-entry bytes leave out what is not got.
    let by_top = plan.bytes_by_top(&manifest(entries()));
    assert_eq!(by_top.get("Films"), None);
    assert_eq!(by_top.get("Music"), Some(&1));
}

#[test]
fn an_empty_choice_gets_everything() {
    let out = tempfile::tempdir().unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("Films/a.mkv", b"aaaa")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(plan.files, [FilePlan::Download { replaceable: None }]);
    assert!(plan.not_chosen.is_empty());
}

#[test]
fn a_previews_bytes_per_entry_leave_out_what_is_already_here() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("Music")).unwrap();
    std::fs::write(out.path().join("Music/c.mp3"), b"c").unwrap();
    let entries = vec![
        entry("Films/a.mkv", b"aaaa"),
        entry("Music/c.mp3", b"c"),
        entry("Music/d.mp3", b"dd"),
    ];
    let plan = plan_folder(out.path(), &manifest(entries.clone()), &[], &[], &cipher()).unwrap();
    let by_top = plan.bytes_by_top(&manifest(entries));
    assert_eq!(by_top.get("Films"), Some(&4));
    // `c.mp3` is already here with this content: only `d.mp3` is written.
    assert_eq!(by_top.get("Music"), Some(&2));
}

#[test]
fn what_is_left_out_inside_one_folder_does_not_come_and_the_rest_does() {
    let out = tempfile::tempdir().unwrap();
    let entries = || {
        vec![
            entry("Disk/MUSIC/a.flac", b"aaaa"),
            entry("Disk/MUSIC/deeper/b.flac", b"bb"),
            entry("Disk/MUSICAL/c.flac", b"c"),
            entry("Disk/Pictures/p.jpg", b"ppp"),
            entry("Disk/readme.txt", b"r"),
        ]
    };
    let plan = plan_folder(
        out.path(),
        &manifest(entries()),
        &[],
        &["Disk/MUSIC".to_owned()],
        &cipher(),
    )
    .unwrap();
    assert_eq!(&plan.files[..2], [FilePlan::NotChosen, FilePlan::NotChosen]);
    assert!(plan.files[2..]
        .iter()
        .all(|file| matches!(file, FilePlan::Download { .. })));
    // Told by what was left out, not by the first part ("Disk").
    assert_eq!(plan.not_chosen, [("Disk/MUSIC".to_owned(), 2, 6)]);
    // Bytes to write by each top-level entry and each entry inside one.
    let by_path = plan.bytes_by_top(&manifest(entries()));
    assert_eq!(by_path.get("Disk"), Some(&5));
    assert_eq!(by_path.get("Disk/Pictures"), Some(&3));
    assert_eq!(by_path.get("Disk/MUSIC"), None);
    assert_eq!(by_path.get("Disk/readme.txt"), Some(&1));
}

#[test]
fn a_folder_left_out_whole_still_covers_what_a_choice_inside_it_said() {
    let out = tempfile::tempdir().unwrap();
    // A top-level choice kept from before the stick became one folder, and
    // one inside: both out, each told once.
    let plan = plan_folder(
        out.path(),
        &manifest(vec![
            entry("Films/a.mkv", b"a"),
            entry("Disk/MUSIC/b.flac", b"bb"),
            entry("Disk/new/c.txt", b"c"),
        ]),
        &[],
        &["Films".to_owned(), "Disk/MUSIC".to_owned()],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        plan.not_chosen,
        [("Disk/MUSIC".to_owned(), 1, 2), ("Films".to_owned(), 1, 1)]
    );
    assert!(matches!(plan.files[2], FilePlan::Download { .. }));
}
