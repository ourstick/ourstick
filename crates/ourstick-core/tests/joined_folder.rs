//! What a joined stick's folder holds (migration 0002): "unchanged" means the
//! folder already has every file of the current manifest, not merely that this
//! device has seen that manifest before.

mod common;

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

use common::memory::{publish_one_file, Link, MemoryFetcher, Published};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadError, DownloadEvent, RelayPolicy};
use ourstick_core::{MemoryStore, Store};

/// Downloads into `output`; returns the `unchanged` of the Manifest event, and
/// whether it finished. With `stop_early` it cancels at the first progress,
/// or when the file is here (a small file tells only that, ADR 0016).
async fn get(
    stick: &Published,
    store: &Arc<MemoryStore>,
    output: &Path,
    stop_early: bool,
) -> (bool, bool) {
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    );
    let mut unchanged = None;
    let result = download(
        &fetcher,
        store.clone(),
        &FixedClock::new(2_000),
        &stick.ticket,
        output,
        &RelayPolicy::default(),
        &mut |event| match event {
            DownloadEvent::Manifest { unchanged: now, .. } => {
                unchanged = Some(now);
                ControlFlow::Continue(())
            }
            DownloadEvent::Progress { .. } | DownloadEvent::FileReceived { .. } if stop_early => {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        },
    )
    .await;
    match result {
        Ok(_) => (unchanged.unwrap(), true),
        Err(DownloadError::Cancelled) => (unchanged.unwrap(), false),
        Err(error) => panic!("{error}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_download_is_not_unchanged_when_it_resumes() {
    let stick = publish_one_file(3 * 1024 * 1024).await;
    let store = Arc::new(MemoryStore::new());
    let output = tempfile::tempdir().unwrap();

    assert_eq!(
        get(&stick, &store, output.path(), true).await,
        (false, false)
    );
    // The record was accepted, but the folder does not hold the files yet.
    assert!(store
        .record_floor(&stick.ticket.stick_id)
        .unwrap()
        .is_some());
    assert_eq!(
        get(&stick, &store, output.path(), false).await,
        (false, true)
    );
    // Now it does.
    assert_eq!(
        get(&stick, &store, output.path(), false).await,
        (true, true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn download_into_another_folder_replaces_the_first() {
    let stick = publish_one_file(1024 * 1024).await;
    let store = Arc::new(MemoryStore::new());
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();

    assert_eq!(
        get(&stick, &store, first.path(), false).await,
        (false, true)
    );
    assert_eq!(
        get(&stick, &store, second.path(), false).await,
        (false, true)
    );
    let joined = store.joined_stick(&stick.ticket.stick_id).unwrap().unwrap();
    assert_eq!(joined.output_dir, second.path());
    assert_eq!(
        std::fs::read(second.path().join("movie.bin")).unwrap(),
        stick.data
    );
    // The first folder keeps its files; the stick now points at the second.
    assert!(first.path().join("movie.bin").exists());
    assert_eq!(
        get(&stick, &store, second.path(), false).await,
        (true, true)
    );
    assert_eq!(
        get(&stick, &store, first.path(), false).await,
        (false, true)
    );
}
