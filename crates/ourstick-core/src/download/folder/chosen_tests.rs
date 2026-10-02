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
