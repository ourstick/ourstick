//! The relay policy of ADR 0006 v2: a download never stops and never asks
//! because of the relay; data through a relay is limited to a rate, and the
//! limit no longer applies once a direct path opens.
//!
//! Uses the in-memory transport with an 8 Mbps limit (1 MB/s, half a second of
//! burst): low enough that waiting for the limit, not decrypting, sets the
//! time of a relay transfer.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::memory::{publish_one_file, Link, MemoryFetcher, Published};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadEvent, DownloadReport, RelayPolicy};
use ourstick_core::transport::PathKind;
use ourstick_core::MemoryStore;

const MIB: u64 = 1024 * 1024;
const LIMIT_BITS: u64 = 8_000_000;
const BYTES_PER_SECOND: f64 = LIMIT_BITS as f64 / 8.0;
const BURST_BYTES: f64 = BYTES_PER_SECOND / 2.0;

fn policy() -> RelayPolicy {
    RelayPolicy {
        limit_bits_per_second: Some(LIMIT_BITS),
    }
}

#[derive(Default, Debug)]
struct Seen {
    limited: Vec<u64>,
    paths: Vec<PathKind>,
}

async fn run(stick: &Published, link: Link) -> (DownloadReport, Seen, Duration, u64) {
    let fetcher = MemoryFetcher::new(stick.served.clone(), link);
    let output = tempfile::tempdir().unwrap();
    let mut seen = Seen::default();
    let started = Instant::now();
    let report = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &policy(),
        &mut |event| {
            match event {
                DownloadEvent::RelayLimited { bits_per_second } => {
                    seen.limited.push(bits_per_second)
                }
                DownloadEvent::PathChanged { path } => seen.paths.push(path),
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(
        std::fs::read(output.path().join("movie.bin")).unwrap(),
        stick.data
    );
    let relay = fetcher.local.lock().unwrap().usage.relay;
    (report, seen, elapsed, relay)
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_found_is_not_limited() {
    let stick = publish_one_file(4 * MIB as usize).await;

    let (_, seen, elapsed, relay) = run(
        &stick,
        Link {
            direct: true,
            ..Link::default()
        },
    )
    .await;

    assert_eq!(relay, 0);
    assert!(seen.limited.is_empty(), "{seen:?}");
    // 4 MiB at the limit would take about 3.7 s.
    let at_limit = (4.0 * MIB as f64 - BURST_BYTES) / BYTES_PER_SECOND;
    assert!(elapsed.as_secs_f64() < at_limit / 2.0, "{elapsed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_only_finishes_at_the_limit_and_says_so() {
    let stick = publish_one_file(4 * MIB as usize).await;

    let (report, seen, elapsed, relay) = run(&stick, Link::default()).await;

    // Never stopped, never asked: it finished, all of it through the relay.
    assert_eq!(report.files, 1);
    assert!(relay >= 4 * MIB);
    assert_eq!(seen.limited, vec![LIMIT_BITS], "announced once");
    // At the limit: the relay bytes beyond the burst took their time.
    let least = (relay as f64 - BURST_BYTES) / BYTES_PER_SECOND;
    assert!(
        elapsed.as_secs_f64() >= least * 0.95,
        "{elapsed:?} for {relay} relay bytes, at least {least:.2} s"
    );
    assert!(elapsed.as_secs_f64() < least + 2.0, "{elapsed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_path_found_midway_lifts_the_limit() {
    let stick = publish_one_file(8 * MIB as usize).await;

    // Through the relay for the first 3 MiB, then directly.
    let (_, seen, elapsed, relay) = run(
        &stick,
        Link {
            direct_after: Some(3 * MIB),
            ..Link::default()
        },
    )
    .await;

    assert_eq!(seen.limited, vec![LIMIT_BITS]);
    assert_eq!(seen.paths.last(), Some(&PathKind::Direct), "{seen:?}");
    assert!(relay <= 4 * MIB, "{relay} relay bytes");
    // Relay only, 8 MiB would take about 7.9 s; here only the first 3 MiB
    // were limited (about 2.6 s).
    let relay_only = (8.0 * MIB as f64 - BURST_BYTES) / BYTES_PER_SECOND;
    assert!(
        elapsed.as_secs_f64() < relay_only / 2.0,
        "{elapsed:?}, relay only would be {relay_only:.2} s"
    );
}
