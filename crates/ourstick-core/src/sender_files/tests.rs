use std::io::{Read, Seek, SeekFrom};

use super::*;

fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("a.txt"), b"hello, world").unwrap();
    std::fs::write(dir.path().join("sub/b.bin"), vec![7u8; 100_000]).unwrap();
    dir
}

#[test]
fn local_files_tell_kinds_sizes_and_disks() {
    let dir = folder();
    let files = LocalFiles;
    let file = files.facts(&dir.path().join("a.txt")).unwrap();
    assert!(file.is_file());
    assert_eq!(file.stamp.size, 12);
    assert!(file.mtime_seconds() > 0);
    assert_eq!(file.disk.is_some(), cfg!(unix));
    assert!(files.facts(&dir.path().join("sub")).unwrap().is_folder());
    let missing = files.facts(&dir.path().join("none")).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn a_link_is_told_as_a_link_unless_followed() {
    let dir = folder();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(dir.path().join("a.txt"), &link).unwrap();
    let files = LocalFiles;
    assert_eq!(files.facts(&link).unwrap().kind, FileKind::Link);
    assert!(files.facts_followed(&link).unwrap().is_file());
    let mut entries = files.entries(dir.path()).unwrap();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let kinds: Vec<_> = entries
        .iter()
        .map(|(name, facts)| (name.to_str().unwrap(), facts.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("a.txt", FileKind::File),
            ("link", FileKind::Link),
            ("sub", FileKind::Folder)
        ]
    );
}

#[test]
fn local_files_are_read_by_position() {
    let dir = folder();
    let file = LocalFiles.open(&dir.path().join("a.txt")).unwrap();
    let mut out = [0u8; 5];
    file.read_exact_at(&mut out, 7).unwrap();
    assert_eq!(&out, b"world");
    assert_eq!(file.read_at(&mut out, 12).unwrap(), 0);
    let short = file.read_exact_at(&mut out, 10).unwrap_err();
    assert_eq!(short.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(file.facts().unwrap().stamp.size, 12);
}

#[test]
fn in_order_reads_and_seeks_as_a_file_does() {
    let dir = folder();
    let path = dir.path().join("sub/b.bin");
    let mut in_order = InOrder::new(LocalFiles.open(&path).unwrap());
    let mut all = Vec::new();
    in_order.read_to_end(&mut all).unwrap();
    assert_eq!(all, std::fs::read(&path).unwrap());
    assert_eq!(in_order.seek(SeekFrom::End(-10)).unwrap(), 99_990);
    assert_eq!(in_order.seek(SeekFrom::Current(4)).unwrap(), 99_994);
    let mut rest = Vec::new();
    in_order.read_to_end(&mut rest).unwrap();
    assert_eq!(rest.len(), 6);
    assert!(in_order.seek(SeekFrom::Current(-200_000)).is_err());
    assert_eq!(in_order.seek(SeekFrom::Start(3)).unwrap(), 3);
}
