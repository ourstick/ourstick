//! Files the reader finds in its folder with the stick's content: counted
//! as here, but recorded as written only if `get` wrote them.

use super::tests::{cipher, entry, manifest};
use super::*;

#[test]
fn a_file_not_written_by_get_counts_only_if_its_content_matches() {
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("same"), b"one").unwrap();
    std::fs::write(out.path().join("other"), b"mine").unwrap();
    // The user's own file, of another time than the entry's.
    std::fs::File::options()
        .write(true)
        .open(out.path().join("same"))
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
        .unwrap();
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
    // Not recorded as written: it may be the user's own, its content
    // guessed by the owner. So a later version never replaces it.
    assert!(plan.found.is_empty());
    let later = plan_folder(
        out.path(),
        &manifest(vec![entry("same", b"the owner's now")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(later.files, vec![FilePlan::ChangedHere]);
}
