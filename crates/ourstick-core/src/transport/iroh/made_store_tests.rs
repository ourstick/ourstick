use std::collections::HashMap;
use std::sync::Arc;

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
    let (store, served) = start(
        cipher(),
        cache.path().to_path_buf(),
        tokio::runtime::Handle::current(),
    );
    served.set(ServedFiles {
        layout: StickLayout::new(folder.path(), &[]),
        files: table,
    });
    Stick {
        folder,
        cache,
        store,
        served,
        blobs,
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
    assert!(stick.store.tags().list().await.is_ok_and(|_| true));
}
