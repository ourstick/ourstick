//! ADR 0016, the reader's side: small files' blobs fetched several at once,
//! a big one alone, twins never twice, "not ready" holding back only its
//! own file, and a stop ending every fetch in flight.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use common::memory::{publish_files, Link, MemoryFetcher};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadError, DownloadEvent, RelayPolicy};
use ourstick_core::MemoryStore;

fn noise(len: usize, seed: u32) -> Vec<u8> {
    (0..len)
        .map(|i| {
            ((i as u32) ^ seed)
                .wrapping_mul(2_654_435_761)
                .to_le_bytes()[1]
        })
        .collect()
}

/// A link whose every fetch waits a little before its data, as a round
/// trip does.
fn slow_link() -> Link {
    Link {
        direct: true,
        latency: Some(Duration::from_millis(20)),
        ..Link::default()
    }
}

/// `count` small files, each different.
fn small_files(count: u32) -> Vec<(String, Vec<u8>)> {
    (0..count)
        .map(|index| (format!("small/{index:03}.txt"), noise(2_000, index + 1)))
        .collect()
}

async fn publish(files: &[(String, Vec<u8>)]) -> common::memory::Published {
    let named: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, data)| (path.as_str(), data.as_slice()))
        .collect();
    publish_files(&named).await
}

fn told(events: &[String], what: &str) -> usize {
    events
        .iter()
        .filter(|event| event.starts_with(what))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn small_files_come_several_at_once_and_all_come() {
    let files = small_files(100);
    let stick = publish(&files).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), slow_link());
    let output = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            match event {
                DownloadEvent::FileStarted { path, .. } => events.push(format!("start {path}")),
                DownloadEvent::FileReceived { path, .. } => events.push(format!("here {path}")),
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
    let served = stick.served.lock().unwrap();
    assert!(served.most_fetching > 1, "one at a time");
    assert!(
        served.most_fetching <= 32,
        "{} at once",
        served.most_fetching
    );
    assert_eq!(served.fetching_now, 0);
    // Told only when here (ADR 0016, rule 3).
    assert_eq!(told(&events, "start"), 0);
    assert_eq!(told(&events, "here"), 100);
    // Started in the plan's order (they may end in any).
    let planned: Vec<[u8; 32]> = stick
        .manifest
        .entries
        .iter()
        .map(|entry| entry.hash)
        .collect();
    let mut first_asked = Vec::new();
    for hash in &served.asked {
        if planned.contains(hash) && !first_asked.contains(hash) {
            first_asked.push(*hash);
        }
    }
    assert_eq!(first_asked, planned);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_big_file_is_fetched_alone_and_told_as_it_comes() {
    // In the plan's order: small/…, then t-big.bin, then zz/….
    let mut files = small_files(10);
    files.push(("t-big.bin".to_owned(), noise(5 << 20, 99)));
    files.extend((10..20).map(|index| (format!("zz/{index:03}.txt"), noise(2_000, index + 1))));
    let stick = publish(&files).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), slow_link());
    let output = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            match event {
                DownloadEvent::FileStarted { path, .. } => events.push(format!("start {path}")),
                DownloadEvent::Progress { path, .. } => events.push(format!("progress {path}")),
                DownloadEvent::FileReceived { path, .. } => events.push(format!("here {path}")),
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
    let big = stick
        .manifest
        .entries
        .iter()
        .find(|entry| entry.path == "t-big.bin")
        .unwrap()
        .hash;
    // Alone: nothing else in flight when it started, after every small file
    // before it was here, and told as it came.
    assert_eq!(stick.served.lock().unwrap().others_at_start[&big], 0);
    let at = |event: &str| events.iter().position(|told| told == event).unwrap();
    for index in 0..10 {
        assert!(at(&format!("here small/{index:03}.txt")) < at("start t-big.bin"));
    }
    assert!(told(&events, "progress t-big.bin") > 0);
    assert_eq!(told(&events, "start"), 1);
    assert!(at("here t-big.bin") < at("here zz/010.txt"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_twin_is_never_fetched_twice() {
    let same = noise(3_000, 7);
    let mut files = small_files(20);
    for index in 0..5 {
        files.push((format!("twins/{index}.txt"), same.clone()));
    }
    let stick = publish(&files).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), slow_link());
    let output = tempfile::tempdir().unwrap();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
    let twin = stick
        .manifest
        .entries
        .iter()
        .find(|entry| entry.path == "twins/0.txt")
        .unwrap()
        .hash;
    let asked = stick
        .served
        .lock()
        .unwrap()
        .asked
        .iter()
        .filter(|asked| **asked == twin)
        .count();
    assert_eq!(asked, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_not_ready_holds_back_only_its_own_file() {
    let files = small_files(20);
    let stick = publish(&files).await;
    let late = stick
        .manifest
        .entries
        .iter()
        .find(|entry| entry.path == "small/000.txt")
        .unwrap()
        .hash;
    stick.served.lock().unwrap().not_ready.insert(late, 2);
    let fetcher = MemoryFetcher::new(stick.served.clone(), slow_link());
    let output = tempfile::tempdir().unwrap();
    let mut here = Vec::new();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if let DownloadEvent::FileReceived { path, .. } = event {
                here.push(path.to_owned());
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
    // The others came while it waited; it came last.
    assert_eq!(here.last().map(String::as_str), Some("small/000.txt"));
    assert_eq!(here.len(), 20);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_fetches_in_flight_ends_them_all_and_the_next_round_resumes() {
    let files = small_files(80);
    let stick = publish(&files).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), slow_link());
    let output = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let mut landed = 0;
    let stopped = download(
        &fetcher,
        store.clone(),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if matches!(event, DownloadEvent::FileReceived { .. }) {
                landed += 1;
                if landed == 5 {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        },
    )
    .await;
    assert!(
        matches!(stopped, Err(DownloadError::Cancelled)),
        "{stopped:?}"
    );
    assert_eq!(stick.served.lock().unwrap().fetching_now, 0);
    // The next round brings the rest.
    download(
        &fetcher,
        store,
        &FixedClock::new(3_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_lost_with_fetches_in_flight_is_made_again_and_the_round_ends() {
    let files = small_files(80);
    let stick = publish(&files).await;
    let link = Link {
        lose_connection_after: Some(40_000),
        ..slow_link()
    };
    let fetcher = MemoryFetcher::new(stick.served.clone(), link);
    let output = tempfile::tempdir().unwrap();
    // Every fetch in flight ends with the connection; the round connects
    // again and resumes them.
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    assert_eq!(stick.served.lock().unwrap().fetching_now, 0);
    assert!(*fetcher.connects.lock().unwrap() >= 2);
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_limit_slows_every_fetch_together() {
    // 600 kB through a relay limited to 100 kB/s, however many fetches go at
    // once. Not all of it is paid for in time: the first half second of the
    // rate is the burst, and the limit is asked every 64 kB (at most every
    // 20 ms, so a crossing may be skipped), so up to two samples at the end
    // arrive with no later question to pause for them: (600 - 50 - 131) kB
    // at 100 kB/s, about 4.2 s. Unlimited, the same takes well under one.
    let files: Vec<(String, Vec<u8>)> = (0..100)
        .map(|index| (format!("small/{index:03}.txt"), noise(6_000, index + 1)))
        .collect();
    let stick = publish(&files).await;
    let link = Link {
        direct: false,
        ..slow_link()
    };
    let fetcher = MemoryFetcher::new(stick.served.clone(), link);
    let output = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy {
            limit_bits_per_second: Some(800_000),
        },
        &mut |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(3_800), "{took:?}");
    for (path, data) in &files {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), data);
    }
}
