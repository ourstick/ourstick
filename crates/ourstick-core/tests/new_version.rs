//! The owner publishes a new version while a reader downloads: the reader
//! goes on with it instead of failing, fetching only what it still needs.
//! A refusal that is not explained by a new version still fails.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::memory::{publish_versions, Link, MemoryFetcher, Versions};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{
    download, DownloadError, DownloadEvent, DownloadReport, RelayPolicy,
};
use ourstick_core::transport::TransportError;
use ourstick_core::MemoryStore;

/// Downloads; when the first file is done, `then` runs (the owner changes
/// something). Returns the result and the new versions reported.
async fn download_while(
    stick: &Versions,
    output: &std::path::Path,
    then: impl Fn() + Sync,
) -> (Result<DownloadReport, DownloadError>, Vec<u64>) {
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    );
    let mut new_versions = Vec::new();
    let mut done = 0;
    let result = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(5_000),
        &stick.ticket,
        output,
        &RelayPolicy::default(),
        &mut |event| {
            match event {
                // The first file's bytes are here (it is placed while the
                // next one is fetched).
                DownloadEvent::FileReceived { .. } => {
                    done += 1;
                    if done == 1 {
                        then();
                    }
                }
                DownloadEvent::NewVersion { seq } => new_versions.push(seq),
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await;
    (result, new_versions)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_version_during_the_download_is_followed() {
    let one = vec![1u8; 300_000];
    let two = vec![2u8; 300_000];
    let two_new = vec![3u8; 300_000];
    let stick = publish_versions(&[
        &[("a.bin", &one), ("b.bin", &two)],
        &[("a.bin", &one), ("b.bin", &two_new), ("c.txt", b"new")],
    ])
    .await;
    let output = tempfile::tempdir().unwrap();

    let (result, new_versions) = download_while(&stick, output.path(), || stick.publish(1)).await;

    let report = result.unwrap();
    assert_eq!(new_versions, vec![2_000]);
    assert_eq!(report.seq, 2_000);
    assert_eq!(report.already_here, 1, "a.bin was not fetched again");
    assert_eq!(std::fs::read(output.path().join("a.bin")).unwrap(), one);
    assert_eq!(std::fs::read(output.path().join("b.bin")).unwrap(), two_new);
    assert_eq!(std::fs::read(output.path().join("c.txt")).unwrap(), b"new");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_without_a_new_version_still_fails() {
    let stick = publish_versions(&[&[("a.bin", &[1u8; 1000]), ("b.bin", &[2u8; 1000])]]).await;
    let output = tempfile::tempdir().unwrap();

    // Same record, but the owner stops serving the blobs.
    let (result, new_versions) =
        download_while(&stick, output.path(), || stick.stop_serving_blobs()).await;

    assert!(
        matches!(
            result,
            Err(DownloadError::Transport(TransportError::Refused))
        ),
        "{result:?}"
    );
    assert!(new_versions.is_empty());
}
