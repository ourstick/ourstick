//! How much a chosen folder or file is (ADR 0028 §2: "counting… 12,400
//! files so far"), as sharing will see it: regular files only, links never
//! followed, the files systems make never counted. A folder that cannot be
//! read is passed over, not an error: this is only for the window. Blocking.

use std::path::Path;

use crate::sender_files::{for_each_file, SenderFiles};

/// Files and bytes counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Measured {
    pub files: u64,
    pub bytes: u64,
}

/// Counts what is at `path`, telling `so_far` after every `every` files.
pub fn measure(
    files: &dyn SenderFiles,
    path: &Path,
    every: u64,
    so_far: &mut dyn FnMut(Measured),
) -> Measured {
    let mut measured = Measured::default();
    let _ = for_each_file::<()>(
        files,
        path,
        &mut |_, facts| {
            measured.files += 1;
            measured.bytes += facts.stamp.size;
            if every > 0 && measured.files % every == 0 {
                so_far(measured);
            }
            Ok(())
        },
        // Passed over: this is only for the window.
        &mut |_, _| Ok(()),
    );
    measured
}

#[cfg(test)]
#[path = "measure_tests.rs"]
mod tests;
