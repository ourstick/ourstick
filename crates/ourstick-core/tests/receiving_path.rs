//! Regression tests from the first real transfers (PC to a MacBook on a phone
//! hotspot, and to the phone itself over mobile data): progress said "relay"
//! while the data arrived directly.
//!
//! 1. With QUIC multipath each side picks the path it sends on; the reader
//!    sends almost nothing and can stay on the relay while the owner sends
//!    everything directly.
//! 2. Hole punching often succeeds while the manifest is fetched, before the
//!    files start; the path display must still switch to "direct".
//!
//! Real iroh on loopback cannot produce either, so these use the in-memory
//! transport.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::memory::{publish_one_file, Link, MemoryFetcher};
use common::NeverAsked;
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadEvent, RelayPolicy};
use ourstick_core::transport::PathKind;
use ourstick_core::MemoryStore;

/// Downloads a 20 MiB stick whose data all arrives directly, and returns the
/// paths the download reported, in order.
async fn reported_paths(link: Link) -> Vec<PathKind> {
    let stick = publish_one_file(20 * 1024 * 1024).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), link);
    let output = tempfile::tempdir().unwrap();
    let mut reported = Vec::new();
    let report = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &NeverAsked,
        &mut |event| {
            if let DownloadEvent::Connected { path } | DownloadEvent::PathChanged { path } = event {
                reported.push(path);
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
    assert!(report.usage.direct >= stick.data.len() as u64);
    reported
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_reports_direct_when_data_arrives_directly_while_it_sends_via_relay() {
    let reported = reported_paths(Link {
        direct: true,
        ..Link::default()
    })
    .await;
    // It starts on the relay (true for the reader's own sending path) ...
    assert_eq!(reported.first(), Some(&PathKind::Relay));
    // ... but all data came directly, so that is what it must end up saying.
    assert_eq!(reported.last(), Some(&PathKind::Direct), "{reported:?}");
}

/// Failed with [Relay] before the tracker started from the reported path.
#[tokio::test(flavor = "multi_thread")]
async fn reader_reports_direct_when_punching_succeeds_before_the_files() {
    let reported = reported_paths(Link {
        punch_during_first_fetch: true,
        ..Link::default()
    })
    .await;
    assert_eq!(reported.first(), Some(&PathKind::Relay));
    assert_eq!(reported.last(), Some(&PathKind::Direct), "{reported:?}");
}
