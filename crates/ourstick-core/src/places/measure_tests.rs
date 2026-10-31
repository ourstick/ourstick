use super::*;

#[test]
fn a_folder_counts_its_files_and_bytes_and_tells_as_it_goes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Holiday 2025");
    std::fs::create_dir_all(root.join("Summer")).unwrap();
    for index in 0..5 {
        std::fs::write(root.join(format!("{index}.jpg")), [0u8; 10]).unwrap();
    }
    std::fs::write(root.join("Summer/sea.jpg"), [0u8; 100]).unwrap();
    // Never sent, so never counted.
    std::fs::write(root.join(".DS_Store"), [0u8; 1000]).unwrap();
    let mut told = Vec::new();
    let measured = measure(&crate::sender_files::LocalFiles, &root, 2, &mut |so_far| {
        told.push(so_far)
    });
    assert_eq!(
        measured,
        Measured {
            files: 6,
            bytes: 150
        }
    );
    assert_eq!(told.len(), 3);
    assert!(told.windows(2).all(|pair| pair[0].files < pair[1].files));
}

#[test]
fn a_file_is_one_and_a_gone_path_is_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, b"hello").unwrap();
    assert_eq!(
        measure(&crate::sender_files::LocalFiles, &notes, 100, &mut |_| {}),
        Measured { files: 1, bytes: 5 }
    );
    assert_eq!(
        measure(
            &crate::sender_files::LocalFiles,
            &dir.path().join("gone"),
            100,
            &mut |_| {}
        ),
        Measured::default()
    );
}

#[cfg(unix)]
#[test]
fn links_inside_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("a");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(dir.path().join("big"), [0u8; 1000]).unwrap();
    std::os::unix::fs::symlink(dir.path().join("big"), root.join("link")).unwrap();
    std::fs::write(root.join("small"), [0u8; 1]).unwrap();
    assert_eq!(
        measure(&crate::sender_files::LocalFiles, &root, 100, &mut |_| {}),
        Measured { files: 1, bytes: 1 }
    );
}
