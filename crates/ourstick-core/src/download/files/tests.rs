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
    Manifest {
        entries,
        ..Manifest::default()
    }
}

fn refused(paths: &[&str]) -> DownloadError {
    check_manifest_paths(&manifest(paths)).unwrap_err()
}

fn passes(paths: &[&str]) {
    check_manifest_paths(&manifest(paths)).unwrap();
}

// 1: names a system refuses: not an error of the stick any more, saved
// under a changed name (saved_paths, ADR 0010); `component_problem` stays
// the final check of every name written.

#[test]
fn drive_like_names_are_refused_by_the_final_check_on_every_platform() {
    for windows in [false, true] {
        for name in ["c:evil", "C:", "z:x", "c:"] {
            assert!(
                component_problem(name, windows).is_some(),
                "{name:?} accepted (windows = {windows})"
            );
        }
    }
    passes(&["c:evil", "sub/c:evil", "C:/Users/victim/evil.exe"]);
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
    passes(&[
        "a.txt",
        "photos/2024/cat.jpg",
        "photos/2024/Dog.jpg",
        ".hidden/file",
        "ünïcödé/ok",
    ]);
}

#[test]
fn names_this_computer_refuses_and_names_one_file_here_pass_the_check() {
    passes(&[
        ".ourstick-partial/x",
        ".OurStick-Received/x",
        "a?.jpg",
        "CON.txt",
    ]);
    passes(&["A.txt", "a.txt", "caf\u{e9}.txt", "cafe\u{301}.txt"]);
    passes(&["a", "a/b"]);
    passes(&[&"a".repeat(256)]);
}

#[test]
fn a_name_too_long_for_the_system_is_refused_by_the_final_check() {
    assert!(component_problem(&"a".repeat(256), false).is_some());
    assert!(component_problem(&"a".repeat(255), false).is_none());
    // 200 Greek letters: 400 bytes, too long on Linux and macOS, not in
    // Windows' units.
    let greek = "α".repeat(200);
    assert!(component_problem(&greek, false).is_some());
    assert!(component_problem(&greek, true).is_none());
}

#[test]
fn a_name_that_is_not_plain_is_refused_and_escaped() {
    for path in ["a/../b", "./x", "a/./b"] {
        assert!(
            matches!(refused(&[path]), DownloadError::UnsafePath { .. }),
            "{path:?}"
        );
    }
    let message = refused(&["\u{1b}[2J/../x"]).to_string();
    assert!(!message.contains('\u{1b}'), "{message:?}");
    assert!(message.contains("\\u{1b}"), "{message:?}");
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
        None,
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
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let attributes = std::fs::metadata(&first).unwrap().file_attributes();
        assert_ne!(attributes & 0x2, 0, "the partial folder is not hidden");
    }
}

#[test]
fn a_file_changed_while_downloading_is_left_alone() {
    let output = tempfile::tempdir().unwrap();
    let partial = prepare_partial_dir(output.path()).unwrap();
    let place = |name: &str, replaceable| {
        let mut blob = Vec::new();
        encrypt_blob(&NoCipher, &mut std::io::Cursor::new(b"data"), &mut blob).unwrap();
        let ciphertext = partial.join(format!("{name}.blob"));
        std::fs::write(&ciphertext, blob).unwrap();
        let entry = Entry {
            path: "file.txt".into(),
            size: 4,
            mtime: 0,
            hash: [0; 32],
        };
        let plaintext = partial.join(format!("{name}.plain"));
        let result = decrypt_into_place(
            &NoCipher,
            &entry,
            &ciphertext,
            &plaintext,
            output.path(),
            replaceable,
        )
        .unwrap();
        assert!(!ciphertext.exists() && !plaintext.exists());
        result
    };
    // Planned as absent, but the user created it meanwhile.
    std::fs::write(output.path().join("file.txt"), b"mine").unwrap();
    assert_eq!(place("a", None), None);
    assert_eq!(
        std::fs::read(output.path().join("file.txt")).unwrap(),
        b"mine"
    );
    // Planned with its stamp and still so: replaced, and the new stamp returned.
    let stamp = FileStamp::of(&std::fs::metadata(output.path().join("file.txt")).unwrap());
    let written = place("b", Some(stamp)).unwrap();
    assert_eq!(
        std::fs::read(output.path().join("file.txt")).unwrap(),
        b"data"
    );
    assert_eq!(written.size, 4);
}

#[test]
fn a_folder_that_is_gone_is_never_made_again() {
    // An unplugged disk's mount point: the stick's folder below it is gone.
    let mount_point = tempfile::tempdir().unwrap();
    let gone = mount_point.path().join("Holiday");
    assert!(matches!(
        prepare_partial_dir(&gone),
        Err(DownloadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));
    assert!(!gone.exists());
}

#[test]
fn a_partial_folder_made_again_by_anyone_else_is_not_this_downloads() {
    let output = tempfile::tempdir().unwrap();
    let partial = prepare_partial_dir(output.path()).unwrap();
    partial_there(&partial).unwrap();
    // The disk went and came back empty; iroh's export made the folder
    // again (create_dir_all): without the mark it is refused.
    std::fs::remove_dir_all(&partial).unwrap();
    std::fs::create_dir_all(&partial).unwrap();
    assert!(matches!(
        partial_there(&partial),
        Err(DownloadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));
}

#[test]
fn a_time_out_of_range_is_not_set_and_the_file_is_kept() {
    let output = tempfile::tempdir().unwrap();
    let partial = prepare_partial_dir(output.path()).unwrap();
    for (name, mtime) in [
        ("far", u64::MAX),
        ("past_i64", 1 << 63),
        ("fine", 1_700_000_000),
    ] {
        let mut blob = Vec::new();
        encrypt_blob(&NoCipher, &mut std::io::Cursor::new(b"data"), &mut blob).unwrap();
        let ciphertext = partial.join(format!("{name}.blob"));
        std::fs::write(&ciphertext, blob).unwrap();
        let entry = Entry {
            path: format!("{name}.txt"),
            size: 4,
            mtime,
            hash: [0; 32],
        };
        let plaintext = partial.join(format!("{name}.plain"));
        decrypt_into_place(
            &NoCipher,
            &entry,
            &ciphertext,
            &plaintext,
            output.path(),
            None,
        )
        .unwrap();
        let saved = output.path().join(format!("{name}.txt"));
        assert_eq!(std::fs::read(&saved).unwrap(), b"data");
        if name == "fine" {
            let modified = std::fs::metadata(&saved).unwrap().modified().unwrap();
            assert_eq!(modified, UNIX_EPOCH + Duration::from_secs(mtime));
        }
    }
}
