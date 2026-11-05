//! A sender's paths, as the reader saves them (ADR 0010): every name
//! savable on each system, none of OurStick's own at the top, no two in one
//! place on the disk. One path a line; only a manifest's valid paths count.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::download::fuzzing::{check_safe_name, check_saved_paths};
use ourstick_core::{Entry, Manifest};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for name in text.split(['\n', '/']) {
        check_safe_name(name);
    }
    let mut paths: Vec<&str> = text.split('\n').take(64).collect();
    paths.sort_unstable();
    paths.dedup();
    let manifest = Manifest {
        entries: paths
            .iter()
            .map(|path| Entry {
                path: (*path).to_owned(),
                size: 0,
                mtime: 0,
                hash: [0; 32],
            })
            .collect(),
        ..Manifest::default()
    };
    if manifest.to_bytes().is_err() {
        return;
    }
    check_saved_paths(&paths);
});
