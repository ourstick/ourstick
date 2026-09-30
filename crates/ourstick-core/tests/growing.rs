//! ADR 0007, the reader's side of a catalogue that is still growing: a
//! partial version does not end a download. The reader takes what is listed,
//! waits for the next version, and goes on until one is complete; such
//! versions are not counted against the limit on new versions.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::growing::grow;
use common::memory::{Link, MemoryFetcher};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, list, DownloadEvent, RelayPolicy};
use ourstick_core::{MemoryStore, Store};

#[tokio::test(flavor = "multi_thread")]
async fn a_download_follows_a_growing_catalogue_until_it_is_complete() {
    let contents: Vec<Vec<u8>> = (0..14u8).map(|i| vec![i; 1000 + i as usize]).collect();
    let names: Vec<String> = (0..14).map(|i| format!("f{i:02}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names
        .iter()
        .zip(&contents)
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    // An empty first version, then one more file each time: 13 partial
    // versions, more than the limit of 10, then the complete one.
    let prefixes: Vec<usize> = (0..=14).collect();
    let growing = grow(&files, &prefixes).await;

    let fetcher = MemoryFetcher::new(growing.served().clone(), Link::default());
    let store = Arc::new(MemoryStore::new());
    let output = tempfile::tempdir().unwrap();
    let mut manifests = Vec::new();
    let mut waits = 0;
    let report = download(
        &fetcher,
        store.clone(),
        &FixedClock::new(2_000),
        &growing.stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            match event {
                DownloadEvent::Manifest {
                    seq,
                    files,
                    partial,
                    ..
                } => manifests.push((seq, files, partial)),
                // The owner catalogues a little more meanwhile.
                DownloadEvent::WaitingForMore { seq } => {
                    waits += 1;
                    growing.publish((seq - 10_000) as usize + 1);
                }
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(waits, 14);
    assert_eq!(manifests.len(), 15);
    assert_eq!(manifests[0], (10_000, 0, true));
    assert_eq!(manifests[14], (10_014, 14, false));
    assert_eq!(report.seq, 10_014);
    assert_eq!(report.files, 14);
    for (name, data) in names.iter().zip(&contents) {
        assert_eq!(&std::fs::read(output.path().join(name)).unwrap(), data);
    }
    // Only the complete version counts as what the folder holds.
    let joined = store
        .joined_stick(&growing.stick.ticket.stick_id)
        .unwrap()
        .unwrap();
    assert_eq!(joined.completed.map(|version| version.seq), Some(10_014));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_says_when_the_catalogue_is_partial() {
    let growing = grow(&[("a", b"one"), ("b", b"two")], &[1, 2]).await;
    let fetcher = MemoryFetcher::new(growing.served().clone(), Link::default());
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let listing = list(&fetcher, store.clone(), &growing.stick.ticket)
        .await
        .unwrap();
    assert!(listing.manifest.partial);
    assert_eq!(listing.manifest.entries.len(), 1);

    growing.publish(1);
    let listing = list(&fetcher, store, &growing.stick.ticket).await.unwrap();
    assert!(!listing.manifest.partial);
    assert_eq!(listing.manifest.entries.len(), 2);
}
