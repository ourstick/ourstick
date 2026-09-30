//! ADR 0007, the reader's side: files arrive batch by batch, each batch's
//! store goes once its files are in place, and a file whose blob another
//! entry already brought is copied, not fetched again.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::memory::{publish_files, Link, MemoryFetcher};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{download, DownloadEvent, RelayPolicy};
use ourstick_core::MemoryStore;

fn noise(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[0] ^ seed)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn batches_leave_once_their_files_are_in_place_and_twins_are_copied() {
    let twin = noise(512 * 1024, 1);
    let other = noise(256 * 1024, 2);
    let stick = publish_files(&[
        ("a/first.bin", &twin),
        ("b/other.bin", &other),
        ("c/same-as-first.bin", &twin),
    ])
    .await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let output = tempfile::tempdir().unwrap();
    let mut copied = Vec::new();
    let report = download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if let DownloadEvent::FileCopiedHere { path, .. } = event {
                copied.push(path.to_owned());
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    for (path, contents) in [
        ("a/first.bin", &twin),
        ("b/other.bin", &other),
        ("c/same-as-first.bin", &twin),
    ] {
        assert_eq!(&std::fs::read(output.path().join(path)).unwrap(), contents);
    }
    assert_eq!(copied, vec!["c/same-as-first.bin"]);
    // The twin's blob came once: well under the three files' blobs.
    assert!(
        report.received < (twin.len() * 2 + other.len()) as u64,
        "{} received",
        report.received
    );
    // Only the manifest's store is left; every file batch was dropped.
    assert_eq!(fetcher.batch_names(), vec!["manifest"]);
    assert!(!fetcher.local.lock().unwrap().dropped.is_empty());
}

/// ADR 0007: a blob the owner is still preparing is "not ready", not a
/// failure. The reader says so once per file, waits and asks again.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_not_ready_yet_is_waited_for() {
    let first = noise(64 * 1024, 3);
    let second = noise(32 * 1024, 4);
    let stick = publish_files(&[("first.bin", &first), ("second.bin", &second)]).await;
    {
        let mut served = stick.served.lock().unwrap();
        let manifest = stick.ticket.manifest_hint.hash;
        let files: Vec<_> = served
            .blobs
            .keys()
            .filter(|hash| **hash != manifest)
            .copied()
            .collect();
        assert_eq!(files.len(), 2);
        for hash in files {
            served.not_ready.insert(hash, 2);
        }
    }
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
    let output = tempfile::tempdir().unwrap();
    let mut preparing = Vec::new();
    download(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &FixedClock::new(2_000),
        &stick.ticket,
        output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if let DownloadEvent::SenderPreparing { path } = event {
                preparing.push(path.to_owned());
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();

    assert_eq!(preparing, vec!["first.bin", "second.bin"]);
    assert_eq!(
        std::fs::read(output.path().join("first.bin")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read(output.path().join("second.bin")).unwrap(),
        second
    );
    assert!(stick
        .served
        .lock()
        .unwrap()
        .not_ready
        .values()
        .all(|left| *left == 0));
}

/// A file is placed while the next one is received: the next one starts
/// (and arrives) before the one before it is reported in place.
#[tokio::test(flavor = "multi_thread")]
async fn the_next_file_is_received_while_one_is_placed() {
    let first = noise(200_000, 5);
    let second = noise(100_000, 6);
    let stick = publish_files(&[("a.bin", &first), ("b.bin", &second)]).await;
    let fetcher = MemoryFetcher::new(stick.served.clone(), Link::default());
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
                DownloadEvent::FileDone { path, .. } => events.push(format!("placed {path}")),
                _ => {}
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    assert_eq!(
        events,
        vec![
            "start a.bin",
            "here a.bin",
            "start b.bin",
            "here b.bin",
            "placed a.bin",
            "placed b.bin"
        ]
    );
    assert_eq!(std::fs::read(output.path().join("a.bin")).unwrap(), first);
    assert_eq!(std::fs::read(output.path().join("b.bin")).unwrap(), second);
}
