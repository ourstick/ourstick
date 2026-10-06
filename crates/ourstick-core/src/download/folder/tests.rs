use std::path::PathBuf;

use super::*;
use crate::cipher::{ChaChaCipher, StickKey};
use crate::manifest::Entry;

pub(super) fn cipher() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([3; 32]))
}

/// The manifest entry the stick would have for `content` at `path`.
pub(super) fn entry(path: &str, content: &[u8]) -> Entry {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    std::fs::write(&file, content).unwrap();
    let mut entry = hash_file(dir.path(), &file, &cipher()).unwrap();
    entry.path = path.to_owned();
    entry
}

pub(super) fn manifest(entries: Vec<Entry>) -> Manifest {
    Manifest {
        entries,
        ..Manifest::default()
    }
}

/// Writes `content` at `path` in `dir` and returns the row `get` would record.
fn written(dir: &Path, path: &str, content: &[u8]) -> DownloadedFile {
    let target: PathBuf = dir.join(path);
    std::fs::write(&target, content).unwrap();
    DownloadedFile {
        path: path.to_owned(),
        saved_as: path.to_owned(),
        stamp: FileStamp::of(&std::fs::metadata(&target).unwrap()),
        blob_hash: entry(path, content).hash,
    }
}

fn later() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

#[test]
fn missing_files_are_downloaded() {
    let out = tempfile::tempdir().unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"one")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(plan.files, vec![FilePlan::Download { replaceable: None }]);
}

#[test]
fn a_file_as_get_left_it_is_skipped_or_replaced_by_a_new_version() {
    let out = tempfile::tempdir().unwrap();
    let row = written(out.path(), "a", b"one");
    let same = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"one")]),
        std::slice::from_ref(&row),
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(same.files, vec![FilePlan::AlreadyHere]);
    let newer = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"two")]),
        std::slice::from_ref(&row),
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        newer.files,
        vec![FilePlan::Download {
            replaceable: Some(row.stamp)
        }]
    );
}

#[test]
fn a_file_changed_here_is_never_overwritten() {
    let out = tempfile::tempdir().unwrap();
    let row = written(out.path(), "a", b"one");
    later();
    std::fs::write(out.path().join("a"), b"my own edit").unwrap();
    for version in [b"one".as_slice(), b"two"] {
        let plan = plan_folder(
            out.path(),
            &manifest(vec![entry("a", version)]),
            std::slice::from_ref(&row),
            &[],
            &cipher(),
        )
        .unwrap();
        assert_eq!(plan.files, vec![FilePlan::ChangedHere]);
    }
}

#[test]
fn a_file_only_touched_here_is_still_updated() {
    let out = tempfile::tempdir().unwrap();
    let row = written(out.path(), "a", b"one");
    later();
    // Same content written again: new times, same bytes.
    std::fs::write(out.path().join("a"), b"one").unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"two")]),
        &[row],
        &[],
        &cipher(),
    )
    .unwrap();
    assert!(matches!(
        plan.files[0],
        FilePlan::Download {
            replaceable: Some(_)
        }
    ));
}

#[test]
fn a_file_not_written_by_get_counts_only_if_its_content_matches() {
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("same"), b"one").unwrap();
    std::fs::write(out.path().join("other"), b"mine").unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("other", b"two"), entry("same", b"one")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        plan.files,
        vec![FilePlan::ChangedHere, FilePlan::AlreadyHere]
    );
    assert_eq!(plan.found.len(), 1);
    assert_eq!(plan.found[0].path, "same");
    assert_eq!(plan.found[0].blob_hash, entry("same", b"one").hash);
}

#[test]
fn a_folder_in_the_way_is_left_alone_and_the_file_saved_beside_it() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("a")).unwrap();
    let stick = manifest(vec![entry("a", b"one")]);
    let plan = plan_folder(out.path(), &stick, &[], &[], &cipher()).unwrap();
    assert_eq!(plan.files, vec![FilePlan::Download { replaceable: None }]);
    assert_eq!(plan.saved.all(&stick), ["a (2)"]);
    assert!(out.path().join("a").is_dir());
}

#[test]
fn files_no_longer_in_the_stick_are_counted_not_touched() {
    let out = tempfile::tempdir().unwrap();
    let gone = written(out.path(), "gone", b"x");
    let deleted_here = written(out.path(), "deleted", b"y");
    std::fs::remove_file(out.path().join("deleted")).unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![]),
        &[gone, deleted_here],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(plan.no_longer_in_stick, 1);
    assert!(out.path().join("gone").exists());
}

#[test]
fn a_file_may_be_replaced_only_as_planned() {
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("a");
    assert!(may_replace(&target, None));
    let row = written(out.path(), "a", b"one");
    assert!(!may_replace(&target, None), "appeared since the plan");
    assert!(may_replace(&target, Some(row.stamp)));
    later();
    std::fs::write(&target, b"edited meanwhile").unwrap();
    assert!(!may_replace(&target, Some(row.stamp)));
}

const WINDOWS: Rules = Rules {
    windows: true,
    fold_case: true,
    fold_normal: false,
};

/// The plan with Windows' rules, as a Windows receiver makes it, and every
/// entry's place.
fn windows_plan(
    dir: &Path,
    entries: Vec<Entry>,
    recorded: &[DownloadedFile],
) -> (FolderPlan, Vec<String>) {
    let stick = manifest(entries);
    let plan = plan_folder_by(dir, &stick, recorded, &[], &cipher(), WINDOWS).unwrap();
    let places = plan
        .saved
        .all(&stick)
        .into_iter()
        .map(str::to_owned)
        .collect();
    (plan, places)
}

#[test]
fn a_windows_receiver_saves_every_file_under_a_name_it_can() {
    let out = tempfile::tempdir().unwrap();
    let (plan, places) = windows_plan(
        out.path(),
        vec![
            entry("CON.txt", b"c"),
            entry("Trip?/x", b"x"),
            entry("a?.jpg", b"q"),
            entry("a_.jpg", b"genuine"),
        ],
        &[],
    );
    assert_eq!(places, ["CON_.txt", "Trip_/x", "a_ (2).jpg", "a_.jpg"]);
    assert!(plan
        .files
        .iter()
        .all(|file| *file == FilePlan::Download { replaceable: None }));
    assert_eq!(plan.renamed.len(), 3);
    assert!(plan.forget.is_empty());
}

#[test]
fn later_versions_keep_the_places_and_a_renamed_folder_takes_new_files() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("Trip_")).unwrap();
    let mut x = written(out.path(), "Trip_/x", b"x");
    x.path = "Trip?/x".to_owned();
    let mut q = written(out.path(), "a_.jpg", b"q");
    q.path = "a?.jpg".to_owned();
    // The sender changes `a?.jpg`, adds `Trip?/y` and a genuine `a_.jpg`.
    let (plan, places) = windows_plan(
        out.path(),
        vec![
            entry("Trip?/x", b"x"),
            entry("Trip?/y", b"y"),
            entry("a?.jpg", b"changed"),
            entry("a_.jpg", b"genuine"),
        ],
        &[x, q],
    );
    assert_eq!(places, ["Trip_/x", "Trip_/y", "a_.jpg", "a_ (2).jpg"]);
    assert_eq!(plan.files[0], FilePlan::AlreadyHere);
    assert!(matches!(
        plan.files[2],
        FilePlan::Download {
            replaceable: Some(_)
        }
    ));
}

#[test]
fn a_renamed_file_that_left_the_stick_is_counted_and_its_place_kept() {
    let out = tempfile::tempdir().unwrap();
    let mut q = written(out.path(), "a_.jpg", b"q");
    q.path = "a?.jpg".to_owned();
    let gone = DownloadedFile {
        path: "old?.txt".to_owned(),
        saved_as: "old_.txt".to_owned(),
        ..q.clone()
    };
    let (plan, places) = windows_plan(out.path(), vec![entry("a_.jpg", b"genuine")], &[q, gone]);
    // The renamed copy is still there: counted, and its place stays its.
    assert_eq!(plan.no_longer_in_stick, 1);
    assert_eq!(places, ["a_ (2).jpg"]);
    // The row of the file that is gone is forgotten.
    assert_eq!(plan.forget, ["old?.txt"]);
}

#[test]
fn the_disk_is_asked_inside_the_folder_whether_it_folds_case() {
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("Notes.txt"), b"n").unwrap();
    let folds = out.path().join("notes.txt").exists() || out.path().join("NOTES.TXT").exists();
    assert_eq!(case_folded(out.path()), folds || cfg!(windows));
    // A folder not made yet is asked through the nearest one above it.
    assert_eq!(
        case_folded(&out.path().join("not/yet")),
        folds || cfg!(windows)
    );
    // Nothing to ask with: folded, the careful answer.
    let empty = tempfile::tempdir().unwrap();
    let digits = empty.path().join("123");
    std::fs::create_dir(&digits).unwrap();
    std::fs::create_dir(digits.join("456")).unwrap();
    assert!(case_folded(&digits.join("456")));
}

#[test]
fn a_file_where_a_received_folder_was_stops_nothing() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("Trip")).unwrap();
    let row = written(out.path(), "Trip/a.jpg", b"a");
    // The user deleted the folder and saved a note of that name.
    std::fs::remove_dir_all(out.path().join("Trip")).unwrap();
    std::fs::write(out.path().join("Trip"), b"my note").unwrap();
    let stick = manifest(vec![entry("Trip/a.jpg", b"a"), entry("b.txt", b"b")]);
    let plan = plan_folder(out.path(), &stick, &[row], &[], &cipher()).unwrap();
    assert_eq!(plan.saved.all(&stick), ["Trip (2)/a.jpg", "b.txt"]);
    assert_eq!(plan.forget, ["Trip/a.jpg"]);
    assert_eq!(std::fs::read(out.path().join("Trip")).unwrap(), b"my note");
}

#[test]
fn a_file_with_no_place_left_is_left_out_and_the_others_planned() {
    let out = tempfile::tempdir().unwrap();
    // Files of the user's at `a` and at every numbered place after it, where
    // the stick's folder `a` must go.
    std::fs::write(out.path().join("a"), b"mine").unwrap();
    for number in 2..=1_000 {
        std::fs::write(out.path().join(format!("a ({number})")), b"mine").unwrap();
    }
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a/b", b"b"), entry("c", b"c")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        plan.files,
        vec![FilePlan::LeftOut, FilePlan::Download { replaceable: None }]
    );
    assert!(plan.renamed.is_empty());
    assert_eq!(
        plan.bytes_to_write(&manifest(vec![entry("a/b", b"b"), entry("c", b"c")])),
        1
    );
}

#[cfg(unix)]
#[test]
fn a_shortcut_at_a_files_place_leaves_only_that_file_out() {
    let out = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("x"), out.path().join("a.txt")).unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a.txt", b"a"), entry("b.txt", b"b")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        plan.files,
        vec![FilePlan::LeftOut, FilePlan::Download { replaceable: None }]
    );
}

#[cfg(unix)]
#[test]
fn a_shortcut_at_a_folders_place_is_never_gone_through() {
    let out = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), out.path().join("a")).unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a/b.txt", b"b"), entry("a/c/d.txt", b"d")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    // The folder is saved beside the link, under another name.
    assert_eq!(plan.renamed.len(), 1, "{:?}", plan.renamed);
    assert_eq!(plan.renamed[0].path, "a");
    assert_ne!(plan.renamed[0].saved_as, "a");
    assert_eq!(plan.renamed[0].files, 2);
}
