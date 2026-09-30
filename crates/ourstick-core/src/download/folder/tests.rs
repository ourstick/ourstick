use std::path::PathBuf;

use super::*;
use crate::cipher::{ChaChaCipher, StickKey};
use crate::manifest::Entry;

fn cipher() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([3; 32]))
}

/// The manifest entry the stick would have for `content` at `path`.
fn entry(path: &str, content: &[u8]) -> Entry {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    std::fs::write(&file, content).unwrap();
    let (mut entry, _) = hash_file(dir.path(), &file, &cipher(), None).unwrap();
    entry.path = path.to_owned();
    entry
}

fn manifest(entries: Vec<Entry>) -> Manifest {
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
        &cipher(),
    )
    .unwrap();
    assert_eq!(same.files, vec![FilePlan::AlreadyHere]);
    let newer = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"two")]),
        std::slice::from_ref(&row),
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
fn a_folder_in_the_way_is_left_alone() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("a")).unwrap();
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a", b"one")]),
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(plan.files, vec![FilePlan::ChangedHere]);
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
