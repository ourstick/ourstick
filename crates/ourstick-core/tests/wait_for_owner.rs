//! `wait_for_owner` (ADR 0025): a link pasted while its owner was away is
//! looked at again, as a receive waits, until the owner answers.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::memory::{publish_one_file, Link, MemoryFetcher};
use ourstick_core::download::{wait_for_owner, Cancel, DownloadError, Waiting};
use ourstick_core::MemoryStore;

const MIB: usize = 1024 * 1024;

fn fetcher(stick: &common::memory::Published) -> MemoryFetcher {
    MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    )
}

fn folder_for(name: &str) -> PathBuf {
    PathBuf::from("/nowhere/Downloads").join(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_who_answers_gives_the_preview_at_once() {
    let stick = publish_one_file(MIB).await;
    let mut waits = Vec::new();
    let preview = wait_for_owner(
        &fetcher(&stick),
        Arc::new(MemoryStore::new()),
        &stick.ticket,
        folder_for,
        Duration::from_secs(5),
        &Cancel::new(),
        &mut |why: &Waiting| waits.push(why.clone()),
    )
    .await
    .unwrap();
    assert_eq!(preview.folder, folder_for(&preview.name));
    assert_eq!(preview.entries.len(), 1);
    assert!(waits.is_empty(), "{waits:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_away_is_waited_for_as_a_receive_does_until_cancelled() {
    let stick = publish_one_file(MIB).await;
    stick.served.lock().unwrap().failed_connects = 1_000;
    let cancel = Cancel::new();
    let cancelled = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel();
        })
    };
    let mut waits = Vec::new();
    let started = Instant::now();
    let waited = wait_for_owner(
        &fetcher(&stick),
        Arc::new(MemoryStore::new()),
        &stick.ticket,
        folder_for,
        Duration::from_secs(5),
        &cancel,
        &mut |why: &Waiting| waits.push(why.clone()),
    )
    .await;
    cancelled.await.unwrap();
    assert!(
        matches!(waited, Err(DownloadError::Cancelled)),
        "{waited:?}"
    );
    // Told why once, then paused (15 s first), and the cancel ends the
    // pause at once.
    assert!(
        matches!(waits.as_slice(), [Waiting::Sender(_)]),
        "{waits:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_who_never_answers_within_the_attempt_counts_as_away() {
    let stick = publish_one_file(MIB).await;
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            latency: Some(Duration::from_secs(60)),
            ..Link::default()
        },
    );
    let cancel = Cancel::new();
    let mut waits = Vec::new();
    let waited = wait_for_owner(
        &fetcher,
        Arc::new(MemoryStore::new()),
        &stick.ticket,
        folder_for,
        Duration::from_millis(200),
        &cancel,
        &mut |why: &Waiting| {
            waits.push(why.clone());
            cancel.cancel();
        },
    )
    .await;
    assert!(
        matches!(waited, Err(DownloadError::Cancelled)),
        "{waited:?}"
    );
    assert!(
        matches!(waits.as_slice(), [Waiting::Sender(_)]),
        "{waits:?}"
    );
}
