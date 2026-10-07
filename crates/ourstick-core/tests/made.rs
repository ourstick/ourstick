//! ADR 0017, two peers: the owner serves its files' blobs from its own
//! store, made from the files when asked, with no encrypted copies and no
//! suitcases at all; the reader gets every file whole, small and big.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{keep_going, noise, Owner, Reader};
use ourstick_core::cipher::cipher_for;
use ourstick_core::share::served_files;
use ourstick_core::transport::Publisher;
use ourstick_core::{CipherId, Store};

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_comes_whole_from_blobs_made_when_asked() {
    let big = noise(3 * 1024 * 1024 + 7, 1);
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|index| {
            (
                format!("small/{index:03}.txt"),
                noise(100 + index * 37, index as u8),
            )
        })
        .chain([("big.bin".to_owned(), big)])
        .chain([("empty".to_owned(), Vec::new())])
        .collect();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    // No keeper: nothing would be served but the manifest.
    let owner = Owner::bare(&listed).await;
    let publication = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();
    let table = served_files(
        owner.store.as_ref(),
        &stick_id,
        std::sync::Arc::new(publication.manifest.clone()),
    )
    .unwrap();
    assert_eq!(table.files.len(), files.len());
    owner.publisher.serve_files(
        cipher_for(CipherId::XChaCha20Poly1305, &owner.stick_key),
        table,
    );

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes, "{name}");
    }
    // No suitcase was ever made.
    let suitcases = owner.publisher.cache_dir().join("suitcases");
    assert_eq!(std::fs::read_dir(suitcases).map_or(0, |dir| dir.count()), 0);
    owner.publisher.shutdown().await;
}

fn files_of(count: usize) -> Vec<(String, Vec<u8>)> {
    (0..count)
        .map(|index| {
            (
                format!("d{}/{index:03}.bin", index % 4),
                noise(500 + index * 911, index as u8),
            )
        })
        .collect()
}

/// Spawns the owner's files keeper (ADR 0017) on `owner`'s publisher; the
/// count of passes it asked for, and its task.
fn keep_files(owner: &Owner) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let passes = Arc::new(AtomicUsize::new(0));
    let counting = passes.clone();
    let keeper = ourstick_core::share::keep_files(
        owner.publisher.clone(),
        owner.store.clone(),
        owner.signing_key.stick_id(),
        owner.stick_key.clone(),
        common::disks(),
        Arc::new(move || {
            counting.fetch_add(1, Ordering::Relaxed);
        }),
    );
    let task = tokio::spawn(async move {
        keeper.await.unwrap();
    });
    (passes, task)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_files_keeper_serves_each_version() {
    let files = files_of(60);
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::bare(&listed).await;
    let publication = owner.publish().await;
    keep_files(&owner);
    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes, "{name}");
    }
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn files_from_an_old_cache_are_made_ready_when_asked() {
    // As after an upgrade: no outboards, no plaintext hashes recorded. The
    // keeper makes them (readers told "not ready" meanwhile) and serves.
    let files = files_of(40);
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::bare(&listed).await;
    let publication = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();
    let _ = std::fs::remove_dir_all(owner.publisher.cache_dir().join("outboards"));
    let old: Vec<_> = owner
        .store
        .owned_files(&stick_id)
        .unwrap()
        .into_iter()
        .map(|file| ourstick_core::store::OwnedFile {
            plaintext_hash: None,
            ..file
        })
        .collect();
    owner.store.record_owned_files(&stick_id, &old).unwrap();
    keep_files(&owner);
    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes, "{name}");
    }
    // Made ready, and recorded so for next time.
    assert!(owner
        .store
        .owned_files(&stick_id)
        .unwrap()
        .iter()
        .all(|file| file.plaintext_hash.is_some()));
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_after_its_version_is_not_sent_until_the_next() {
    let files = files_of(10);
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::bare(&listed).await;
    let first = owner.publish().await;
    let (passes, _) = keep_files(&owner);
    // Same size, other bytes: the version still lists the old blob.
    let (name, bytes) = &files[3];
    let mut changed = bytes.clone();
    changed[bytes.len() / 2] ^= 0xff;
    std::fs::write(owner.folder.path().join(name), &changed).unwrap();

    let reader = Reader::new();
    let stuck = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.download(&owner.ticket(&first), &mut keep_going),
    )
    .await;
    assert!(
        !matches!(stuck, Ok(Ok(_))),
        "a file that changed was received under its old version"
    );
    assert!(!reader.exists(name) || reader.read(name) != changed);
    // The keeper asked the watcher for a pass.
    assert!(passes.load(Ordering::Relaxed) > 0);

    // The next pass reads it again; its version brings the new file.
    let second = owner.publish().await;
    let again = Reader::new();
    again
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(again.read(name), changed);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_keeper_ends_with_the_publisher() {
    let owner = Owner::bare(&[("a", b"a")]).await;
    owner.publish().await;
    let (_, task) = keep_files(&owner);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    owner.publisher.clone().shutdown().await;
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("the keeper did not end")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_version_with_no_new_blob_is_served_too() {
    // A rename: the same blob at a new path, nothing new to ask for.
    let owner = Owner::bare(&[("old.txt", b"same"), ("b.txt", b"b")]).await;
    owner.publish().await;
    keep_files(&owner);
    std::fs::rename(
        owner.folder.path().join("old.txt"),
        owner.folder.path().join("new.txt"),
    )
    .unwrap();
    let second = owner.publish().await;
    let reader = Reader::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        reader.download(&owner.ticket(&second), &mut keep_going),
    )
    .await
    .expect("still waiting")
    .unwrap();
    assert_eq!(reader.read("new.txt"), b"same");
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_files_are_made_once_and_recorded_at_each_path() {
    let same = noise(40_000, 9);
    let owner = Owner::bare(&[("one.bin", same.as_slice()), ("two.bin", same.as_slice())]).await;
    let publication = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();
    let old: Vec<_> = owner
        .store
        .owned_files(&stick_id)
        .unwrap()
        .into_iter()
        .map(|file| ourstick_core::store::OwnedFile {
            plaintext_hash: None,
            ..file
        })
        .collect();
    owner.store.record_owned_files(&stick_id, &old).unwrap();
    keep_files(&owner);
    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("two.bin"), same);
    let recorded = owner.store.owned_files(&stick_id).unwrap();
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().all(|file| file.plaintext_hash.is_some()));
}
