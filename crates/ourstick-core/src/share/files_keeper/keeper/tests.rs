use std::path::Path;

use super::*;
use crate::manifest::Entry;

fn listed(path: &str, hash: u8, served: bool) -> Listed {
    Listed {
        entry: Entry {
            path: path.to_owned(),
            size: 10,
            mtime: 0,
            hash: [hash; 32],
        },
        served,
        stamp: None,
    }
}

/// A keeper of the version `[version; 32]` listing `entries`.
fn keeper(version: u8, entries: Vec<Listed>) -> Keeper {
    let mut keeper = Keeper::default();
    keeper.learn(
        [version; 32],
        StickLayout::new(Path::new("/x"), &[]),
        entries,
    );
    keeper
}

fn next_hash(keeper: &mut Keeper) -> Option<BlobHash> {
    keeper.next().map(|blob| blob.hash)
}

#[test]
fn files_not_served_are_made_in_manifest_order() {
    let mut keeper = keeper(
        1,
        vec![
            listed("a", 1, false),
            listed("b", 2, true),
            listed("c", 3, false),
        ],
    );
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
    assert_eq!(next_hash(&mut keeper), Some([3; 32]));
    assert_eq!(next_hash(&mut keeper), None);
}

#[test]
fn a_file_a_reader_waits_on_goes_first() {
    let mut keeper = keeper(
        1,
        vec![
            listed("a", 1, false),
            listed("b", 2, false),
            listed("c", 3, false),
        ],
    );
    assert!(keeper.asked([3; 32]));
    assert!(!keeper.asked([9; 32]), "not of this version");
    assert_eq!(next_hash(&mut keeper), Some([3; 32]));
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
}

#[test]
fn identical_files_are_one_blob_at_every_path() {
    let mut keeper = keeper(1, vec![listed("a", 1, false), listed("b", 1, false)]);
    let blob = keeper.next().unwrap();
    assert_eq!(blob.paths, ["a", "b"]);
    assert_eq!(blob.stamps.len(), 2);
    assert!(keeper.next().is_none());
}

#[test]
fn a_file_that_failed_waits_longer_each_time() {
    let mut keeper = keeper(1, vec![listed("a", 1, false)]);
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
    keeper.failed_now([1; 32]);
    assert_eq!(keeper.failed[&[1; 32]].1, FIRST_RETRY);
    // Not due yet: not made again, even when a reader asks at once.
    keeper.retry_due();
    assert!(keeper.asked([1; 32]));
    assert_eq!(next_hash(&mut keeper), None);
    keeper.failed_now([1; 32]);
    assert_eq!(keeper.failed[&[1; 32]].1, FIRST_RETRY * 2);
    for _ in 0..10 {
        keeper.failed_now([1; 32]);
    }
    assert_eq!(keeper.failed[&[1; 32]].1, LONGEST_RETRY);
}

#[test]
fn a_retry_due_is_queued_once_while_it_is_made() {
    let mut keeper = keeper(1, vec![listed("a", 1, false)]);
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
    keeper.failed_now([1; 32]);
    // Its wait is over.
    let long_ago = Instant::now() - LONGEST_RETRY;
    keeper.failed.get_mut(&[1; 32]).unwrap().0 = long_ago;
    keeper.retry_due();
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
    // Being made: the next looks queue nothing.
    keeper.retry_due();
    keeper.retry_due();
    assert_eq!(next_hash(&mut keeper), None);
}

#[test]
fn a_reader_waiting_on_a_failed_file_has_it_made_once_its_first_wait_is_over() {
    let mut keeper = keeper(1, vec![listed("a", 1, false), listed("b", 2, false)]);
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
    keeper.failed_now([1; 32]);
    keeper.failed_now([1; 32]);
    // Tried more than FIRST_RETRY ago, though its wait (a minute) is not over.
    keeper.failed.get_mut(&[1; 32]).unwrap().0 = Instant::now() - FIRST_RETRY;
    assert!(keeper.asked([1; 32]));
    assert_eq!(next_hash(&mut keeper), Some([1; 32]));
}

#[test]
fn a_new_version_starts_afresh() {
    let mut keeper = keeper(1, vec![listed("a", 1, false), listed("b", 2, false)]);
    keeper.asked([2; 32]);
    keeper.failed_now([1; 32]);
    keeper.learn(
        [2; 32],
        StickLayout::new(Path::new("/x"), &[]),
        vec![listed("c", 1, false)],
    );
    assert_eq!(keeper.manifest_hash, Some([2; 32]));
    assert!(keeper.failed.is_empty());
    let blob = keeper.next().unwrap();
    assert_eq!(blob.paths, ["c"]);
    assert!(keeper.next().is_none());
}
