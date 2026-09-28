use super::*;
use crate::blob::encrypt_blob;
use crate::cipher::NoCipher;

fn manifest(paths: &[&str]) -> Manifest {
    let mut entries: Vec<Entry> = paths
        .iter()
        .map(|path| Entry {
            path: (*path).to_owned(),
            size: 0,
            mtime: 0,
            hash: [0; 32],
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Manifest { entries }
}

fn refused(paths: &[&str]) -> DownloadError {
    check_manifest_paths(&manifest(paths)).unwrap_err()
}

// 1: names that would leave the output folder or be misread on Windows.

#[test]
fn drive_like_names_are_refused_on_every_platform() {
    for windows in [false, true] {
        for name in ["c:evil", "C:", "z:x", "c:"] {
            assert!(
                component_problem(name, windows).is_some(),
                "{name:?} accepted (windows = {windows})"
            );
        }
    }
    for path in ["c:evil", "sub/c:evil", "C:/Users/victim/evil.exe"] {
        assert!(
            matches!(refused(&[path]), DownloadError::UnsafePath { .. }),
            "{path:?}"
        );
    }
}

#[test]
fn windows_rules_refuse_invalid_and_reserved_names() {
    let refused_on_windows = [
        "a:b",
        "a<b",
        "a>b",
        "a\"b",
        "a|b",
        "a?b",
        "a*b",
        "a\u{1}b",
        "name.",
        "name ",
        "...",
        "CON",
        "con",
        "Con.txt",
        "NUL",
        "nul.tar.gz",
        "PRN",
        "AUX",
        "COM1",
        "com9.log",
        "LPT1",
        "lpt9",
    ];
    for name in refused_on_windows {
        assert!(
            component_problem(name, true).is_some(),
            "{name:?} accepted on Windows"
        );
    }
    // Fine on Windows too.
    for name in [
        "COM0",
        "COM10",
        "CONSOLE",
        "nullable",
        ".hidden",
        "..a",
        "a..b",
        "ünïcödé",
        "a b",
    ] {
        assert_eq!(
            component_problem(name, true),
            None,
            "{name:?} refused on Windows"
        );
    }
    // Elsewhere only drive-like names are refused.
    for name in ["a<b", "CON", "name.", "...", "ünïcödé", "a b"] {
        assert_eq!(
            component_problem(name, false),
            None,
            "{name:?} refused off Windows"
        );
    }
}

#[test]
fn ordinary_paths_pass() {
    check_manifest_paths(&manifest(&[
        "a.txt",
        "photos/2024/cat.jpg",
        "photos/2024/Dog.jpg",
        ".hidden/file",
        "ünïcödé/ok",
    ]))
    .unwrap();
}

// 5: our partial directory, in any letter case.

#[test]
fn partial_directory_is_refused_in_any_case() {
    for path in [
        ".ourstick-partial/x",
        ".OURSTICK-PARTIAL/x",
        ".OurStick-Partial/deep/x",
        ".ourstick-partial",
    ] {
        assert!(
            matches!(refused(&[path]), DownloadError::UnsafePath { .. }),
            "{path:?}"
        );
    }
    // Only as the first component; deeper it is an ordinary name.
    check_manifest_paths(&manifest(&["sub/.ourstick-partial/x"])).unwrap();
}

// 3: paths that land on one file when letter case is ignored.

#[test]
fn case_collisions_are_refused() {
    assert!(matches!(
        refused(&["A.txt", "a.txt"]),
        DownloadError::PathCollision { .. }
    ));
    assert!(matches!(
        refused(&["Photos/cat.jpg", "photos/cat.jpg"]),
        DownloadError::PathCollision { .. }
    ));
}

#[test]
fn a_file_that_is_also_a_folder_is_refused() {
    for paths in [
        &["a", "a/b"][..],
        &["A", "a/b"],
        &["x/y", "x/y/z"],
        &["X/Y", "x/y/z"],
    ] {
        assert!(
            matches!(refused(paths), DownloadError::PathCollision { .. }),
            "{paths:?}"
        );
    }
    // A shared folder is fine.
    check_manifest_paths(&manifest(&["a/b", "a/c"])).unwrap();
}

#[test]
fn paths_in_errors_are_escaped() {
    let nasty = "\u{1b}[2J/c:evil";
    let message = refused(&[nasty]).to_string();
    assert!(!message.contains('\u{1b}'), "{message:?}");
    assert!(message.contains("\\u{1b}"), "{message:?}");

    let message = refused(&["A\u{1b}", "a\u{1b}"]).to_string();
    assert!(!message.contains('\u{1b}'), "{message:?}");
}

// 2: symbolic links, at the start and again right before the move.

#[cfg(unix)]
#[test]
fn target_path_refuses_existing_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    assert!(matches!(
        target_path(dir.path(), "link/escape.txt"),
        Err(DownloadError::UnsafePath { .. })
    ));
    assert_eq!(
        target_path(dir.path(), "a/b.txt").unwrap(),
        dir.path().join("a").join("b.txt")
    );
}

#[cfg(unix)]
#[test]
fn symlink_planted_after_the_first_check_is_caught_before_the_move() {
    let output = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let partial = prepare_partial_dir(output.path()).unwrap();

    // First check passes: nothing there yet.
    target_path(output.path(), "sub/file.txt").unwrap();
    // Then someone swaps in a symbolic link while the download runs.
    std::os::unix::fs::symlink(outside.path(), output.path().join("sub")).unwrap();

    let mut blob = Vec::new();
    encrypt_blob(&NoCipher, &mut std::io::Cursor::new(b"data"), &mut blob).unwrap();
    let ciphertext = partial.join("x.blob");
    std::fs::write(&ciphertext, blob).unwrap();
    let entry = Entry {
        path: "sub/file.txt".into(),
        size: 4,
        mtime: 0,
        hash: [0; 32],
    };
    let result = decrypt_into_place(
        &NoCipher,
        &entry,
        &ciphertext,
        &partial.join("x.plain"),
        output.path(),
    );
    assert!(
        matches!(result, Err(DownloadError::UnsafePath { .. })),
        "{result:?}"
    );
    assert!(
        !outside.path().join("file.txt").exists(),
        "wrote through the link"
    );
}

#[cfg(unix)]
#[test]
fn partial_directory_that_is_a_symlink_is_refused() {
    let output = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), output.path().join(PARTIAL_DIR)).unwrap();
    assert!(matches!(
        prepare_partial_dir(output.path()),
        Err(DownloadError::UnsafePath { .. })
    ));
}

#[test]
fn partial_directory_is_created_and_reused() {
    let output = tempfile::tempdir().unwrap();
    let first = prepare_partial_dir(output.path()).unwrap();
    let second = prepare_partial_dir(output.path()).unwrap();
    assert_eq!(first, second);
    assert!(first.is_dir());
}
