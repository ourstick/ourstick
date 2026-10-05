use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bao_tree::io::mixed::EncodedItem;
use futures::StreamExt;

use super::*;
use crate::blob::encrypt_blob;
use crate::cipher::{ChaChaCipher, StickKey};
use crate::layout::StickLayout;
use crate::manifest::hashing::hash_file_telling_plaintext;
use crate::transport::ServedFile;

fn cipher() -> Arc<dyn Cipher> {
    Arc::new(ChaChaCipher::new(&StickKey::from_bytes([6; 32])))
}

fn content(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i * 13 + 1) as u8).collect()
}

/// A stick folder with `files`, catalogued: its store, and each file's
/// blob hash and blob.
struct Stick {
    folder: tempfile::TempDir,
    cache: tempfile::TempDir,
    store: Store,
    served: Served,
    blobs: Vec<([u8; 32], Vec<u8>)>,
    /// What the store told it could not serve.
    told: Arc<Mutex<Vec<(Hash, bool)>>>,
}

fn stick(files: &[(&str, Vec<u8>)]) -> Stick {
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut table = HashMap::new();
    let mut blobs = Vec::new();
    for (name, data) in files {
        let path = folder.path().join(name);
        std::fs::write(&path, data).unwrap();
        let hashed = hash_file_telling_plaintext(
            (*name).to_owned(),
            &path,
            cipher().as_ref(),
            None,
            Some(cache.path()),
        )
        .unwrap();
        let mut blob = Vec::new();
        encrypt_blob(cipher().as_ref(), &mut io::Cursor::new(data), &mut blob).unwrap();
        blobs.push((hashed.entry.hash, blob));
        table.insert(
            hashed.entry.hash,
            ServedFile {
                path: (*name).into(),
                plaintext_len: data.len() as u64,
                plaintext_hash: hashed.plaintext_hash,
            },
        );
    }
    let told = Arc::new(Mutex::new(Vec::new()));
    let telling = told.clone();
    let (store, served) = start(
        cipher(),
        cache.path().to_path_buf(),
        tokio::runtime::Handle::current(),
        Arc::new(move |hash: Hash, changed| telling.lock().unwrap().push((hash, changed))),
    );
    served.set(
        cipher(),
        ServedFiles {
            layout: StickLayout::new(folder.path(), &[]),
            files: table,
        },
        |_| {},
    );
    Stick {
        folder,
        cache,
        store,
        served,
        blobs,
        told,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_is_served_as_encrypt_blob_wrote_it() {
    let stick = stick(&[
        ("small.txt", content(300)),
        ("big.bin", content(200 * 1024 + 5)),
    ]);
    for (hash, blob) in &stick.blobs {
        let got = stick
            .store
            .export_bao(Hash::from_bytes(*hash), ChunkRanges::all())
            .data_to_vec()
            .await
            .unwrap();
        assert_eq!(&got, blob);
    }
    assert_eq!(
        stick.served.blob_len(&Hash::from_bytes(stick.blobs[1].0)),
        Some(stick.blobs[1].1.len() as u64)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_file_never_sends_a_byte_that_differs() {
    // Small (one group, checked against the root) and big (by the tree).
    for size in [5000, 300 * 1024] {
        let original = content(size);
        let stick = stick(&[("f", original.clone())]);
        let (hash, blob) = &stick.blobs[0];
        let mut changed = original.clone();
        changed[size * 2 / 3] ^= 0xff;
        std::fs::write(stick.folder.path().join("f"), &changed).unwrap();
        let mut items = stick
            .store
            .export_bao(Hash::from_bytes(*hash), ChunkRanges::all())
            .stream()
            .boxed();
        let mut sent = 0usize;
        let mut ended_with_error = false;
        while let Some(item) = items.next().await {
            match item {
                EncodedItem::Leaf(leaf) => {
                    let at = leaf.offset as usize;
                    assert_eq!(&leaf.data[..], &blob[at..at + leaf.data.len()]);
                    sent += leaf.data.len();
                }
                EncodedItem::Error(_) => ended_with_error = true,
                EncodedItem::Size(told) => assert_eq!(told, blob.len() as u64),
                _ => {}
            }
        }
        assert!(ended_with_error, "size {size}");
        assert!(sent < blob.len(), "size {size}");
        // Told as changed, and no longer served.
        let told = stick.told.lock().unwrap().clone();
        assert_eq!(told, vec![(Hash::from_bytes(*hash), true)], "size {size}");
        assert_eq!(stick.served.blob_len(&Hash::from_bytes(*hash)), None);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_its_outboard_or_record_a_blob_is_not_served() {
    let stick = stick(&[("big.bin", content(100 * 1024))]);
    let hash = Hash::from_bytes(stick.blobs[0].0);
    std::fs::remove_file(outboard_path(stick.cache.path(), hash.as_bytes())).unwrap();
    assert!(stick
        .store
        .export_bao(hash, ChunkRanges::all())
        .data_to_vec()
        .await
        .is_err());
    // A short outboard is refused, not read.
    std::fs::write(outboard_path(stick.cache.path(), hash.as_bytes()), b"short").unwrap();
    assert!(stick
        .store
        .export_bao(hash, ChunkRanges::all())
        .data_to_vec()
        .await
        .is_err());
    // A blob it does not serve.
    assert!(stick
        .store
        .export_bao(Hash::from_bytes([7; 32]), ChunkRanges::all())
        .data_to_vec()
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn other_commands_are_refused() {
    let stick = stick(&[("a", content(10))]);
    assert!(stick.store.add_bytes(vec![1u8, 2, 3]).await.is_err());
    assert!(stick
        .store
        .tags()
        .set("t", Hash::from_bytes(stick.blobs[0].0))
        .await
        .is_err());
    assert!(stick.store.blobs().batch().await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_range_is_served_alone_as_for_a_resume() {
    let stick = stick(&[("big.bin", content(500 * 1024))]);
    let (hash, blob) = &stick.blobs[0];
    // The chunks of the second half only (1 KiB chunks).
    let half = (blob.len() / 2 / 1024) as u64;
    let ranges = ChunkRanges::from(bao_tree::ChunkNum(half)..);
    let mut items = stick
        .store
        .export_bao(Hash::from_bytes(*hash), ranges)
        .stream()
        .boxed();
    let mut first = None;
    let mut done = false;
    while let Some(item) = items.next().await {
        match item {
            EncodedItem::Leaf(leaf) => {
                let at = leaf.offset as usize;
                first.get_or_insert(at);
                assert_eq!(&leaf.data[..], &blob[at..at + leaf.data.len()]);
            }
            EncodedItem::Done => done = true,
            EncodedItem::Error(error) => panic!("{error}"),
            _ => {}
        }
    }
    assert!(done);
    assert!(first.is_some_and(|at| at as u64 >= half * 1024 - 16 * 1024));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_outboard_of_the_right_length_sends_nothing() {
    let stick = stick(&[("big.bin", content(100 * 1024))]);
    let hash = Hash::from_bytes(stick.blobs[0].0);
    let path = outboard_path(stick.cache.path(), hash.as_bytes());
    let mut outboard = std::fs::read(&path).unwrap();
    outboard[0] ^= 1;
    std::fs::write(&path, outboard).unwrap();
    let mut items = stick
        .store
        .export_bao(hash, ChunkRanges::all())
        .stream()
        .boxed();
    while let Some(item) = items.next().await {
        assert!(
            !matches!(item, EncodedItem::Leaf(_) | EncodedItem::Done),
            "something was sent"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_table_replaces_the_old_one() {
    let stick = stick(&[("a", content(10))]);
    let old = Hash::from_bytes(stick.blobs[0].0);
    stick.served.set(
        cipher(),
        ServedFiles {
            layout: StickLayout::new(stick.folder.path(), &[]),
            files: HashMap::new(),
        },
        |_| {},
    );
    assert!(stick
        .store
        .export_bao(old, ChunkRanges::all())
        .data_to_vec()
        .await
        .is_err());
    assert_eq!(stick.served.blob_len(&old), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_cannot_be_read_is_told_unread_not_changed() {
    let stick = stick(&[("gone.txt", content(400))]);
    let hash = Hash::from_bytes(stick.blobs[0].0);
    std::fs::remove_file(stick.folder.path().join("gone.txt")).unwrap();
    assert!(stick
        .store
        .export_bao(hash, ChunkRanges::all())
        .data_to_vec()
        .await
        .is_err());
    assert_eq!(stick.told.lock().unwrap().clone(), vec![(hash, false)]);
    // A blob not in the version is not told about.
    let _ = stick
        .store
        .export_bao(Hash::from_bytes([8; 32]), ChunkRanges::all())
        .data_to_vec()
        .await;
    assert_eq!(stick.told.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_added_are_served_too() {
    let stick = stick(&[("a", content(10))]);
    let extra = stick.folder.path().join("b");
    std::fs::write(&extra, content(70 * 1024)).unwrap();
    let hashed = hash_file_telling_plaintext(
        "b".to_owned(),
        &extra,
        cipher().as_ref(),
        None,
        Some(stick.cache.path()),
    )
    .unwrap();
    stick.served.add(
        vec![(
            hashed.entry.hash,
            ServedFile {
                path: "b".into(),
                plaintext_len: 70 * 1024,
                plaintext_hash: hashed.plaintext_hash,
            },
        )],
        |_| {},
    );
    let mut blob = Vec::new();
    encrypt_blob(
        cipher().as_ref(),
        &mut io::Cursor::new(content(70 * 1024)),
        &mut blob,
    )
    .unwrap();
    let got = stick
        .store
        .export_bao(Hash::from_bytes(hashed.entry.hash), ChunkRanges::all())
        .data_to_vec()
        .await
        .unwrap();
    assert_eq!(got, blob);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_that_goes_away_is_no_fault_of_the_blob() {
    let stick = stick(&[("big.bin", content(400 * 1024))]);
    let hash = Hash::from_bytes(stick.blobs[0].0);
    {
        let mut items = stick
            .store
            .export_bao(hash, ChunkRanges::all())
            .stream()
            .boxed();
        // The first item, then the reader is gone.
        let _ = items.next().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(stick.told.lock().unwrap().is_empty());
    // Still served.
    let got = stick
        .store
        .export_bao(hash, ChunkRanges::all())
        .data_to_vec()
        .await
        .unwrap();
    assert_eq!(got, stick.blobs[0].1);
}
