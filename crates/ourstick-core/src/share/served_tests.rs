use super::*;
use crate::cipher::CipherId;
use crate::manifest::{Entry, FileStamp};
use crate::stick_id::StickSigningKey;
use crate::store::{MemoryStore, OwnedFile, OwnedStick};

fn entry(path: &str, hash: u8) -> Entry {
    Entry {
        path: path.to_owned(),
        size: 10,
        mtime: 0,
        hash: [hash; 32],
    }
}

fn record(path: &str, hash: u8, plaintext: Option<u8>) -> OwnedFile {
    OwnedFile {
        path: path.to_owned(),
        stamp: FileStamp {
            size: 10,
            mtime_ns: 0,
            ctime_ns: 0,
        },
        blob_hash: [hash; 32],
        plaintext_hash: plaintext.map(|byte| [byte; 32]),
    }
}

#[test]
fn only_files_recorded_with_this_blob_and_a_plaintext_hash_are_served() {
    let store = MemoryStore::new();
    let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
    store
        .add_owned_stick(&OwnedStick::new(stick_id, "s", "/stick", CipherId::None, 0))
        .unwrap();
    store
        .record_owned_files(
            &stick_id,
            &[
                record("a", 1, Some(11)),
                // Recorded before plaintext hashes were kept.
                record("b", 2, None),
                // Recorded with another blob (the file changed since).
                record("c", 9, Some(13)),
            ],
        )
        .unwrap();
    let manifest = Manifest {
        partial: false,
        name: String::new(),
        entries: vec![entry("a", 1), entry("b", 2), entry("c", 3), entry("d", 4)],
    };
    let served = served_files(&store, &stick_id, Arc::new(manifest)).unwrap();
    assert_eq!(served.files.len(), 1);
    let a = &served.files[&[1; 32]];
    assert_eq!(a.index, 0);
    assert_eq!(served.entry(a).path, "a");
    assert_eq!(a.plaintext_hash, [11; 32]);
    assert_eq!(served.layout.on_disk("a"), std::path::Path::new("/stick/a"));
}
