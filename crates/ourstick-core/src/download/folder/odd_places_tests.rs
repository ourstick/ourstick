//! Places the reader cannot use for one file: that file is left out, the
//! others come, and nothing waits for good.

use super::tests::{cipher, entry, manifest};
use super::*;

#[test]
fn a_path_too_long_for_this_system_leaves_only_that_file_out() {
    let out = tempfile::tempdir().unwrap();
    // Valid in the manifest (under 4 096 bytes), too long with the folder
    // in front: Linux takes at most 255 bytes a name, 4 096 a path.
    let long = format!("{}/{}", "d".repeat(200), "n".repeat(250));
    let mut deep = [long.as_str(); 9].join("/") + "/";
    deep.push_str(&"f".repeat(4090 - deep.len()));
    assert!(deep.len() <= crate::manifest::MAX_PATH_BYTES);
    assert!(out.path().join(&deep).as_os_str().len() > 4096);
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("b.txt", b"b"), entry(&deep, b"x")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_eq!(
        plan.files,
        vec![FilePlan::Download { replaceable: None }, FilePlan::LeftOut]
    );
}

#[test]
fn a_named_pipe_at_a_files_place_is_never_read() {
    let out = tempfile::tempdir().unwrap();
    let pipe = out.path().join("a.txt");
    let made = std::process::Command::new("mkfifo").arg(&pipe).status();
    if !made.is_ok_and(|status| status.success()) {
        return;
    }
    // Reading a pipe with no writer would block this for good.
    let plan = plan_folder(
        out.path(),
        &manifest(vec![entry("a.txt", b"a"), entry("b.txt", b"b")]),
        &[],
        &[],
        &cipher(),
    )
    .unwrap();
    assert_ne!(plan.files[0], FilePlan::Download { replaceable: None });
    assert_eq!(plan.files[1], FilePlan::Download { replaceable: None });
}
