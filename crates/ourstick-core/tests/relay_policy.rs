//! The relay policy of ADR 0006: small transfers may use the relay; larger ones
//! wait for a direct path, and only go through the relay if the user agrees,
//! also after a direct path is lost mid-transfer.
//!
//! Uses the in-memory transport, with a small "free" limit and a short wait so
//! the tests are fast.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use common::memory::{publish_one_file, Link, MemoryConsent, MemoryFetcher, Published};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{
    download, DownloadError, DownloadEvent, DownloadReport, RelayPolicy,
};
use ourstick_core::MemoryStore;

const MIB: u64 = 1024 * 1024;

/// Relay free up to 1 MiB, wait 300 ms for a direct path.
fn test_policy() -> RelayPolicy {
    RelayPolicy {
        free_relay_bytes: MIB,
        direct_wait: Duration::from_millis(300),
    }
}

#[derive(Default, Debug)]
struct Seen {
    waited: usize,
    direct_lost: usize,
}

async fn run(
    stick: &Published,
    fetcher: &MemoryFetcher,
    policy: RelayPolicy,
    consent: &MemoryConsent,
    output: &std::path::Path,
) -> (Result<DownloadReport, DownloadError>, Seen) {
    let mut seen = Seen::default();
    let result = download(
        fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output,
        &policy,
        consent,
        &mut |event| {
            match event {
                DownloadEvent::WaitingForDirect { .. } => seen.waited += 1,
                DownloadEvent::DirectLost => seen.direct_lost += 1,
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await;
    (result, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_found_downloads_without_asking() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    );
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    let report = result.unwrap();
    assert!(consent.asked.lock().unwrap().is_empty());
    assert_eq!(seen.waited, 1);
    assert_eq!(report.usage.relay, 0);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_direct_path_and_user_says_no_stops_before_any_file_data() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, _) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    assert!(
        matches!(result, Err(DownloadError::RelayDeclined { remaining }) if remaining == 20 * MIB),
        "{result:?}"
    );
    let asked = consent.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].remaining, 20 * MIB);
    assert_eq!(asked[0].relay, "the test relay");
    assert!(!asked[0].reasons.is_empty());
    assert!(!asked[0].direct_was_lost);
    // Only the manifest went through the relay, no file data.
    let relay_bytes = fetcher.local.lock().unwrap().usage.relay;
    assert!(
        relay_bytes < 64 * 1024,
        "{relay_bytes} bytes through the relay"
    );
    assert!(!output.path().join("movie.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_direct_path_and_user_says_yes_downloads_through_the_relay() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(true);
    let output = tempfile::tempdir().unwrap();

    let (result, _) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    let report = result.unwrap();
    assert_eq!(consent.asked.lock().unwrap().len(), 1);
    assert!(report.usage.relay >= 20 * MIB);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn small_transfer_uses_the_relay_without_waiting_or_asking() {
    // 20 MiB is below the real 100 MB limit: default policy.
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(
        &stick,
        &fetcher,
        RelayPolicy::default(),
        &consent,
        output.path(),
    )
    .await;

    let report = result.unwrap();
    assert!(consent.asked.lock().unwrap().is_empty());
    assert_eq!(seen.waited, 0, "a small transfer does not wait");
    assert!(report.usage.relay >= 20 * MIB);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_lost_mid_transfer_reconnects_then_asks() {
    let stick = publish_one_file(40 * MIB as usize).await;
    // Direct at first; lost for good after 12 MiB.
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            lose_direct_after: Some(12 * MIB),
            ..Link::default()
        },
    );
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    assert!(
        matches!(result, Err(DownloadError::RelayDeclined { .. })),
        "{result:?}"
    );
    assert_eq!(seen.direct_lost, 1, "reconnected once");
    assert_eq!(*fetcher.connects.lock().unwrap(), 2);
    let asked = consent.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].direct_was_lost);
    // At most one check interval (4 MiB) plus a piece went through the relay.
    let relay_bytes = fetcher.local.lock().unwrap().usage.relay;
    assert!(
        relay_bytes <= 5 * MIB + 64 * 1024,
        "{relay_bytes} bytes through the relay"
    );
    // What arrived is kept, so the next run resumes.
    let kept = fetcher.local_bytes_total();
    assert!(kept >= 12 * MIB, "bytes kept for the resume: {kept}");
    assert!(!output.path().join("movie.bin").exists());
}
