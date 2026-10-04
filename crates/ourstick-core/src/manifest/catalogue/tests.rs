use super::*;
use crate::cipher::{ChaChaCipher, StickKey};
use crate::manifest::Manifest;

fn cipher() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([3; 32]))
}

fn write(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn files(part: &[Catalogued]) -> Vec<CataloguedFile> {
    part.iter()
        .map(|item| match item {
            Catalogued::File(file) => file.clone(),
            Catalogued::Skipped { path } => panic!("skipped {path}"),
        })
        .collect()
}

#[test]
fn parts_follow_manifest_order_and_hash_like_a_full_encryption() {
    let dir = tempfile::tempdir().unwrap();
    // Walk order visits the folder "a" first; manifest order puts "a-c"
    // first ('-' < '/').
    write(dir.path(), "a/b", b"in a folder");
    write(dir.path(), "a-c", b"beside it");
    write(dir.path(), "z", &[7; 5000]);

    let mut catalogue = Catalogue::start(dir.path(), Vec::new()).unwrap();
    assert_eq!(catalogue.len(), 3);
    assert_eq!(catalogue.last_done(), None);
    assert_eq!(catalogue.paths(), vec!["a-c", "a/b", "z"]);
    let mut all = files(&catalogue.next_part(&cipher(), 2, u64::MAX, None).unwrap());
    assert_eq!(catalogue.done(), 2);
    assert_eq!(catalogue.last_done(), Some("a/b"));
    assert!(!catalogue.is_finished());
    all.extend(files(
        &catalogue.next_part(&cipher(), 2, u64::MAX, None).unwrap(),
    ));
    assert!(catalogue.is_finished());
    assert!(catalogue
        .next_part(&cipher(), 2, u64::MAX, None)
        .unwrap()
        .is_empty());

    let manifest = Manifest::from_dir(dir.path(), &cipher()).unwrap();
    let entries: Vec<Entry> = all.iter().map(|file| file.entry.clone()).collect();
    assert_eq!(entries, manifest.entries);
    assert!(all.iter().all(|file| file.read));
    assert_eq!(
        all[2].plaintext_hash,
        Some(*blake3::hash(&[7; 5000]).as_bytes())
    );
}

#[test]
fn a_part_stops_once_enough_was_read() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        write(dir.path(), name, &[1; 1000]);
    }
    let mut catalogue = Catalogue::start(dir.path(), Vec::new()).unwrap();
    // The file that reaches the limit is the last of the part.
    assert_eq!(
        catalogue
            .next_part(&cipher(), 10, 1500, None)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        catalogue
            .next_part(&cipher(), 10, 1500, None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_file_as_recorded_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "kept", b"old");
    write(dir.path(), "changed", b"new");
    let mut first = Catalogue::start(dir.path(), Vec::new()).unwrap();
    let mut recorded: Vec<OwnedFile> =
        files(&first.next_part(&cipher(), 10, u64::MAX, None).unwrap())
            .iter()
            .map(CataloguedFile::owned_file)
            .collect();
    // A recorded hash no file would give, to see that it is trusted.
    recorded[1].blob_hash = [9; 32];
    recorded[1].plaintext_hash = None;
    // "changed" has another stamp now.
    recorded[0].stamp.size += 1;

    let mut second = Catalogue::start(dir.path(), recorded).unwrap();
    let part = files(&second.next_part(&cipher(), 10, u64::MAX, None).unwrap());
    assert_eq!(part[0].entry.path, "changed");
    assert!(part[0].read);
    assert_eq!(part[1].entry.path, "kept");
    assert!(!part[1].read);
    assert_eq!(part[1].entry.hash, [9; 32]);
    assert_eq!(part[1].plaintext_hash, None);
    assert_eq!(part[1].entry.size, 3);
}

#[test]
fn a_file_gone_since_the_walk_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "gone", b"x");
    write(dir.path(), "stays", b"y");
    let mut catalogue = Catalogue::start(dir.path(), Vec::new()).unwrap();
    std::fs::remove_file(dir.path().join("gone")).unwrap();
    let part = catalogue.next_part(&cipher(), 10, u64::MAX, None).unwrap();
    assert_eq!(
        part[0],
        Catalogued::Skipped {
            path: "gone".to_owned()
        }
    );
    assert!(matches!(&part[1], Catalogued::File(file) if file.entry.path == "stays"));
}

#[test]
fn recorded_entries_are_those_still_found_in_manifest_order() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "b", b"b");
    write(dir.path(), "a", b"a");
    let recorded = |path: &str, fill: u8| OwnedFile {
        path: path.to_owned(),
        stamp: FileStamp {
            size: 10,
            mtime_ns: 1_700_000_000_900_000_000,
            ctime_ns: 0,
        },
        blob_hash: [fill; 32],
        plaintext_hash: None,
    };
    // "gone" is no longer there; "a" was never recorded.
    let catalogue =
        Catalogue::start(dir.path(), vec![recorded("gone", 9), recorded("b", 2)]).unwrap();
    assert_eq!(
        catalogue.recorded_entries(),
        vec![Entry {
            path: "b".to_owned(),
            size: 10,
            // Whole seconds, as the catalogue records them.
            mtime: 1_700_000_000,
            hash: [2; 32],
        }]
    );
}
