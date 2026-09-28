//! The relay policy of ADR 0006: a short wait for a direct path, then up to a
//! budget through the relay while a direct path is looked for; once the budget
//! is used up, only a direct path, unless the user agrees.
//!
//! Uses the in-memory transport, with an 8 MiB budget and short waits so the
//! tests are fast. The budget is checked every 4 MiB of file data.

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

/// An 8 MiB relay budget, 300 ms waits for a direct path.
fn test_policy() -> RelayPolicy {
    RelayPolicy {
        relay_budget: 8 * MIB,
        first_wait: Duration::from_millis(300),
        budget_wait: Duration::from_millis(300),
    }
}

#[derive(Default, Debug)]
struct Seen {
    /// `budget_used` of each `WaitingForDirect`.
    waited: Vec<bool>,
    budget_used: Vec<u64>,
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
                DownloadEvent::WaitingForDirect { budget_used, .. } => {
                    seen.waited.push(budget_used)
                }
                DownloadEvent::RelayBudgetUsed { bytes } => seen.budget_used.push(bytes),
                DownloadEvent::DirectLost => seen.direct_lost += 1,
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await;
    (result, seen)
}

fn relay_total(fetcher: &MemoryFetcher) -> u64 {
    fetcher.local.lock().unwrap().usage.relay
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_found_downloads_without_waiting_or_asking() {
    let stick = publish_one_file(40 * MIB as usize).await;
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
    assert!(seen.waited.is_empty());
    assert_eq!(relay_total(&fetcher), 0);
    assert!(report.usage.direct >= 40 * MIB);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_direct_path_uses_the_budget_then_the_user_says_no() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    // Waited before the start, then again once the budget was used up.
    assert_eq!(seen.waited, vec![false, true]);
    assert_eq!(seen.budget_used.len(), 1);
    assert_eq!(*fetcher.connects.lock().unwrap(), 2);
    let asked = consent.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let remaining = asked[0].remaining;
    assert!(
        matches!(result, Err(DownloadError::RelayDeclined { remaining: declined }) if declined == remaining),
        "{result:?}"
    );
    assert!(remaining < 12 * MIB, "{remaining} bytes left");
    assert_eq!(asked[0].relay, "the test relay");
    assert!(!asked[0].reasons.is_empty());
    assert!(!asked[0].direct_was_lost);
    // Over the budget by at most one check interval plus a piece.
    let used = asked[0].relay_used;
    assert!(
        used > 8 * MIB && used <= 13 * MIB,
        "{used} bytes through the relay"
    );
    assert_eq!(used, relay_total(&fetcher));
    // What arrived is kept, so the next run resumes.
    let kept = fetcher.local_bytes_total();
    assert!(kept > 8 * MIB, "bytes kept for the resume: {kept}");
    assert!(!output.path().join("movie.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_direct_path_and_user_says_yes_downloads_through_the_relay() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(true);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    let report = result.unwrap();
    assert_eq!(consent.asked.lock().unwrap().len(), 1);
    assert_eq!(seen.budget_used.len(), 1);
    assert!(relay_total(&fetcher) >= 20 * MIB);
    // The report counts both connections, not only the last one.
    assert!(report.received >= 20 * MIB, "received {}", report.received);
    assert_eq!(report.usage.relay, relay_total(&fetcher));
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_within_the_budget_uses_the_relay_without_asking() {
    let stick = publish_one_file(20 * MIB as usize).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();
    let policy = RelayPolicy {
        relay_budget: 32 * MIB,
        ..test_policy()
    };

    let (result, seen) = run(&stick, &fetcher, policy, &consent, output.path()).await;

    let report = result.unwrap();
    assert!(consent.asked.lock().unwrap().is_empty());
    assert_eq!(seen.waited, vec![false], "only the short wait at the start");
    assert!(seen.budget_used.is_empty());
    assert!(report.usage.relay >= 20 * MIB);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_found_while_on_the_relay_finishes_directly() {
    let stick = publish_one_file(40 * MIB as usize).await;
    // Hole punching succeeds after 4 MiB through the relay.
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct_after: Some(4 * MIB),
            ..Link::default()
        },
    );
    let consent = MemoryConsent::answering(false);
    let output = tempfile::tempdir().unwrap();

    let (result, seen) = run(&stick, &fetcher, test_policy(), &consent, output.path()).await;

    let report = result.unwrap();
    assert!(consent.asked.lock().unwrap().is_empty());
    assert!(seen.budget_used.is_empty());
    assert_eq!(*fetcher.connects.lock().unwrap(), 1);
    let relay = relay_total(&fetcher);
    assert!(relay <= 5 * MIB, "{relay} bytes through the relay");
    assert!(report.usage.direct >= 35 * MIB);
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_lost_mid_transfer_continues_within_the_budget_then_asks() {
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
    // No special handling: the relay budget covered the loss, then ran out.
    assert_eq!(seen.direct_lost, 0);
    assert_eq!(seen.budget_used.len(), 1);
    let asked = consent.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].direct_was_lost);
    let relay = relay_total(&fetcher);
    assert!(
        relay > 8 * MIB && relay <= 13 * MIB,
        "{relay} bytes through the relay"
    );
    assert!(fetcher.local_bytes_total() >= 20 * MIB);
    assert!(!output.path().join("movie.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_lost_after_the_budget_reconnects_then_asks() {
    let stick = publish_one_file(40 * MIB as usize).await;
    // Relay at first; direct after 10 MiB (too late: the budget is used up by
    // the next check); lost for good after 20 MiB.
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct_after: Some(10 * MIB),
            lose_direct_after: Some(20 * MIB),
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
    assert_eq!(seen.budget_used.len(), 1);
    assert_eq!(seen.direct_lost, 1);
    assert_eq!(*fetcher.connects.lock().unwrap(), 3);
    let asked = consent.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].direct_was_lost);
    // After the budget, at most one check interval plus a piece more.
    let relay = relay_total(&fetcher);
    assert!(relay <= 16 * MIB, "{relay} bytes through the relay");
    assert!(!output.path().join("movie.bin").exists());
}
