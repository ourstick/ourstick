//! A lost connection does not end a download: a new one goes on where it
//! was, and the reader says so. Seen for real: a reader on Wi-Fi lost its
//! connection in the middle of a 979 GB stick and stopped. A phone tells
//! its endpoint when the network changes.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::memory::{publish_one_file, Link, MemoryFetcher};
use common::{keep_going, Owner, Reader};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadEvent, RelayPolicy};
use ourstick_core::transport::{Fetcher, Publisher};
use ourstick_core::MemoryStore;

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_connection_is_made_again_and_the_download_goes_on() {
    let stick = publish_one_file(5 * 1024 * 1024).await;
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            lose_connection_after: Some(2 * 1024 * 1024),
            ..Link::default()
        },
    );
    let output = tempfile::tempdir().unwrap();
    let mut lost = Vec::new();
    let report = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if let DownloadEvent::ConnectionLost { reason } = event {
                lost.push(reason.to_owned());
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(lost, vec!["read: connection lost"]);
    assert_eq!(report.files, 1);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
    // Resumed: what arrived before the loss was not fetched again.
    assert!(
        report.received < stick.data.len() as u64 + 100_000,
        "{}",
        report.received
    );
    assert_eq!(*fetcher.connects.lock().unwrap(), 2);
}

/// Harmless, before and after: the endpoint goes on working (ADR 0027).
#[tokio::test(flavor = "multi_thread")]
async fn a_network_change_leaves_the_endpoint_working() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    fetcher.network_change().await;
    let report = download(
        &fetcher,
        reader.store.clone(),
        &reader.clock,
        &ticket,
        reader.output.path(),
        &RelayPolicy::default(),
        &mut keep_going,
    )
    .await
    .unwrap();
    assert_eq!(report.files, 1);
    assert_eq!(reader.read("a.txt"), b"hello");
    fetcher.network_change().await;
    fetcher.shutdown(true).await.unwrap();
    owner.publisher.shutdown().await;
}
