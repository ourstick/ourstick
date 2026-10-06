//! A gauge of the plan's memory (ADR 0018), run by hand under heaptrack:
//! `cargo test --release -p ourstick-core --lib plan_of_many_files -- --ignored`.

use super::tests::{cipher, manifest};
use super::*;
use crate::manifest::Entry;

/// `count` files in folders six deep, about 1.4 folders a file, as in a
/// big photo or music folder.
fn deep_stick(count: usize) -> Manifest {
    let entries = (0..count)
        .map(|index| {
            let mut folders = String::new();
            let mut rest = index;
            for _ in 0..6 {
                folders.push_str(&format!("d{}/", rest % 7));
                rest /= 7;
            }
            Entry {
                path: format!("{folders}f{index:06}.txt"),
                size: 1000,
                mtime: 0,
                hash: [1; 32],
            }
        })
        .collect::<Vec<_>>();
    let mut entries = entries;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    manifest(entries)
}

#[test]
#[ignore = "a gauge, run by hand under heaptrack"]
fn plan_of_many_files() {
    let stick = deep_stick(100_000);
    let dir = tempfile::tempdir().unwrap();
    let plan = plan_folder(dir.path(), &stick, &[], &[], &cipher()).unwrap();
    assert_eq!(plan.files.len(), 100_000);
}
