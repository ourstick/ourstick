//! How much a chosen folder or file is (ADR 0028 §2: "counting… 12,400
//! files so far"), as sharing will see it: regular files only, links never
//! followed, the files systems make never counted. A folder that cannot be
//! read is passed over, not an error: this is only for the window. Blocking.

use std::path::Path;

use walkdir::WalkDir;

use crate::manifest::ignore::is_ignored;

/// Files and bytes counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Measured {
    pub files: u64,
    pub bytes: u64,
}

/// Counts what is at `path`, telling `so_far` after every `every` files.
pub fn measure(path: &Path, every: u64, so_far: &mut dyn FnMut(Measured)) -> Measured {
    let mut measured = Measured::default();
    let walk = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|item| {
            item.depth() == 0 || !item.file_name().to_str().is_some_and(is_ignored)
        });
    for item in walk.flatten() {
        if !item.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        measured.files += 1;
        measured.bytes += metadata.len();
        if every > 0 && measured.files % every == 0 {
            so_far(measured);
        }
    }
    measured
}

#[cfg(test)]
#[path = "measure_tests.rs"]
mod tests;
