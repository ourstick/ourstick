use std::sync::Arc;

use super::*;
use crate::cipher::{CipherId, NoCipher};
use crate::manifest::Manifest;
use crate::stick_id::StickSigningKey;
use crate::store::{MemoryStore, OwnedFile, OwnedStick};

/// A file this big needs an outboard (more than one 16 KiB group).
const BIG: u64 = 100_000;

fn entry(path: &str, hash: u8, size: u64) -> Entry {
    Entry {
        path: path.to_owned(),
        size,
        mtime: 0,
        hash: [hash; 32],
    }
}

fn record(path: &str, hash: u8, size: u64, plaintext: Option<u8>) -> OwnedFile {
    OwnedFile {
        path: path.to_owned(),
        stamp: FileStamp {
            size,
            mtime_ns: 0,
            ctime_ns: 0,
        },
        blob_hash: [hash; 32],
        plaintext_hash: plaintext.map(|byte| [byte; 32]),
    }
}

fn load(records: &[OwnedFile], entries: Vec<Entry>, outboards: &Path) -> Version {
    let store = MemoryStore::new();
    let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
    store
        .add_owned_stick(&OwnedStick::new(stick_id, "s", "/stick", CipherId::None, 0))
        .unwrap();
    store.record_owned_files(&stick_id, records).unwrap();
    let manifest = Arc::new(Manifest {
        partial: false,
        name: String::new(),
        entries,
    });
    let published = PublishedVersion {
        manifest_hash: [9; 32],
        manifest,
    };
    load_version(&store, &stick_id, outboards, &NoCipher, published).unwrap()
}

#[test]
fn only_files_recorded_with_this_blob_a_plaintext_hash_and_an_outboard_are_served() {
    let outboards = tempfile::tempdir().unwrap();
    // "f" has the outboard its size needs, "g" none.
    let tree = BaoTree::new(blob_len(&NoCipher, BIG), BLOCK_SIZE);
    let outboard = vec![0; tree.outboard_size() as usize];
    std::fs::write(outboard_path(outboards.path(), &[6; 32]), outboard).unwrap();
    let version = load(
        &[
            record("a", 1, 10, Some(11)),
            // Recorded before plaintext hashes were kept.
            record("b", 2, 10, None),
            // Recorded with another blob (the file changed since).
            record("c", 9, 10, Some(13)),
            record("f", 6, BIG, Some(16)),
            record("g", 7, BIG, Some(17)),
            // A row the manifest does not list.
            record("z", 8, 10, Some(18)),
        ],
        vec![
            entry("a", 1, 10),
            entry("b", 2, 10),
            entry("c", 3, 10),
            // An entry without a row.
            entry("d", 4, 10),
            entry("f", 6, BIG),
            entry("g", 7, BIG),
        ],
        outboards.path(),
    );
    let served: Vec<bool> = version.listed.iter().map(|listed| listed.served).collect();
    assert_eq!(served, [true, false, false, false, true, false]);
    let stamped: Vec<bool> = version
        .listed
        .iter()
        .map(|listed| listed.stamp.is_some())
        .collect();
    assert_eq!(stamped, [true, true, false, false, true, true]);
    let files = &version.served.files;
    assert_eq!(files.len(), 2);
    assert_eq!(files[&[1; 32]].index, 0);
    assert_eq!(files[&[1; 32]].plaintext_hash, [11; 32]);
    assert_eq!(version.served.entry(&files[&[6; 32]]).path, "f");
    assert_eq!(version.manifest_hash, [9; 32]);
    assert_eq!(
        version.served.layout.on_disk("a"),
        std::path::Path::new("/stick/a")
    );
}

#[test]
fn a_blob_at_several_paths_is_served_from_the_first_recorded() {
    let outboards = tempfile::tempdir().unwrap();
    let version = load(
        &[record("b", 1, 10, Some(11)), record("c", 1, 10, Some(11))],
        vec![entry("a", 1, 10), entry("b", 1, 10), entry("c", 1, 10)],
        outboards.path(),
    );
    let served: Vec<bool> = version.listed.iter().map(|listed| listed.served).collect();
    assert_eq!(served, [false, true, true]);
    assert_eq!(version.served.files[&[1; 32]].index, 1);
}
