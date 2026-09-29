//! `list` (`ourstick ls`): the verified manifest, no file data, and nothing
//! written but the record floor.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::memory::{publish_one_file, Link, MemoryFetcher};
use common::NeverAsked;
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, list, DownloadError, DownloadEvent, RelayPolicy};
use ourstick_core::{ManifestRecord, MemoryStore, RecordError, Store};

const MIB: usize = 1024 * 1024;

fn fetcher(stick: &common::memory::Published) -> MemoryFetcher {
    MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn list_shows_the_files_and_fetches_no_file_data() {
    let stick = publish_one_file(4 * MIB).await;
    let fetcher = fetcher(&stick);
    let store = Arc::new(MemoryStore::new());

    let listing = list(&fetcher, store.clone(), &stick.ticket).await.unwrap();

    let entries = &listing.manifest.entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "movie.bin");
    assert_eq!(entries[0].size, 4 * MIB as u64);
    // Only the manifest blob arrived.
    assert!(fetcher.local_bytes_total() < 64 * 1024);
    // The floor is raised; no folder is joined.
    assert_eq!(
        store.record_floor(&stick.ticket.stick_id).unwrap(),
        Some(listing.version)
    );
    assert_eq!(store.joined_stick(&stick.ticket.stick_id).unwrap(), None);
    // Listing again is fine: same record, not "older".
    list(&fetcher, store.clone(), &stick.ticket).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_refuses_a_record_older_than_the_floor() {
    let stick = publish_one_file(MIB).await;
    let store = Arc::new(MemoryStore::new());
    store
        .accept_record(&ManifestRecord {
            stick_id: stick.ticket.stick_id,
            seq: u64::from(u32::MAX) * 1_000,
            manifest_hash: [7; 32],
        })
        .unwrap();

    let result = list(&fetcher(&stick), store, &stick.ticket).await;

    assert!(
        matches!(
            result,
            Err(DownloadError::Record(RecordError::NotNewer { .. }))
        ),
        "{result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_after_list_downloads_everything_and_is_not_unchanged() {
    let stick = publish_one_file(2 * MIB).await;
    let store = Arc::new(MemoryStore::new());
    list(&fetcher(&stick), store.clone(), &stick.ticket)
        .await
        .unwrap();

    let output = tempfile::tempdir().unwrap();
    let mut unchanged = None;
    download(
        &fetcher(&stick),
        store.clone(),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &NeverAsked,
        &mut |event| {
            if let DownloadEvent::Manifest { unchanged: now, .. } = event {
                unchanged = Some(now);
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(unchanged, Some(false));
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}
