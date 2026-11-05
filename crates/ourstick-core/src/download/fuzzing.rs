//! Entry points for the fuzz targets in `fuzz/` (built only with
//! `--cfg fuzzing`, which cargo-fuzz sets): checks of the crate's own rules
//! over code that is not public. Each panics when a rule does not hold.

use std::collections::HashSet;

use super::files::component_problem;
use super::saved_names::{fold_case, not_ours, safe_name};
use super::saved_paths::{saved_paths, OnDisk, Rules};

/// `safe_name` gives a name this system can save, and the same name again
/// when given its own result.
pub fn check_safe_name(name: &str) {
    if name.is_empty() || name.contains('/') {
        return;
    }
    for windows in [false, true] {
        let safe = safe_name(name, windows);
        assert!(
            component_problem(&safe, windows).is_none(),
            "{name:?} saved as {safe:?}, which this system refuses"
        );
        assert_eq!(safe_name(&safe, windows), safe, "{name:?}: not idempotent");
    }
}

/// Where a stick's `paths` (a manifest's, sorted, valid) are saved in an
/// empty folder, under every system's rules: every name savable, none of
/// OurStick's own at the top, no two places one on the disk, no file where
/// a folder is, and nothing left out.
pub fn check_saved_paths(paths: &[&str]) {
    for (windows, fold_case_too, fold_normal) in [
        (false, false, false),
        (false, true, true),
        (true, true, false),
    ] {
        let rules = Rules {
            windows,
            fold_case: fold_case_too,
            fold_normal,
        };
        let planned = saved_paths(paths, &[], &HashSet::new(), |_, _| OnDisk::Nothing, rules);
        assert!(
            planned.left_out.is_empty(),
            "{paths:?}: left out in an empty folder"
        );
        let disk = |path: &str| {
            let cased = if fold_case_too {
                fold_case(path)
            } else {
                path.to_owned()
            };
            if fold_normal {
                icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
                    .normalize(&cased)
                    .into_owned()
            } else {
                cased
            }
        };
        let mut files = HashSet::new();
        let mut folders = HashSet::new();
        for saved in &planned.saved {
            let names: Vec<&str> = saved.split('/').collect();
            for name in &names {
                assert!(
                    component_problem(name, windows).is_none(),
                    "{saved:?}: {name:?} cannot be saved"
                );
            }
            assert!(
                not_ours(
                    names[0],
                    &[super::PARTIAL_DIR, super::receive::RECEIVED_MARKER]
                )
                .is_none(),
                "{saved:?}: one of OurStick's own names"
            );
            let place = disk(saved);
            assert!(
                files.insert(place.clone()),
                "{saved:?}: two files in one place"
            );
            for end in 1..names.len() {
                folders.insert(disk(&names[..end].join("/")));
            }
        }
        for file in &files {
            assert!(
                !folders.contains(file),
                "{file:?}: a file where a folder is"
            );
        }
    }
}
