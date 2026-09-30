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
            if let DownloadEvent::FileDone {
                path, received: 0, ..
            } = event
            {
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
