//! `get` into a folder that already has the stick: only what changed is
//! fetched, files changed in the folder are never overwritten, and files that
//! left the stick are kept.

mod common;

use std::ops::ControlFlow;
use std::sync::Arc;

use common::{Owner, Reader};
use ourstick_core::clock::Clock;
use ourstick_core::download::{DownloadEvent, DownloadReport};
use ourstick_core::transport::Publisher;
use ourstick_core::MemoryStore;

/// Downloads the owner's latest version; returns the report and the
/// "no longer in the stick" count, if reported.
async fn get(owner: &Owner, reader: &Reader) -> (DownloadReport, Option<usize>, Vec<String>) {
    let ticket = owner.ticket(&owner.publish().await);
    let mut gone = None;
    let mut changed_here = Vec::new();
    let report = reader
        .download(&ticket, &mut |event| {
            match event {
                DownloadEvent::NoLongerInStick { count } => gone = Some(count),
                DownloadEvent::FileChangedHere { path, .. } => changed_here.push(path.to_owned()),
                _ => {}
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
    (report, gone, changed_here)
}

fn later() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

#[tokio::test(flavor = "multi_thread")]
async fn only_what_changed_is_written_again() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two"), ("c.txt", b"three")]).await;
    let reader = Reader::new();
    let (first, _, _) = get(&owner, &reader).await;
    assert_eq!(first.already_here, 0);

    // Again, nothing changed: everything is already here.
    let (again, _, _) = get(&owner, &reader).await;
    assert_eq!((again.already_here, again.changed_here), (3, 0));

    later();
    std::fs::write(owner.folder.path().join("b.txt"), b"two, changed").unwrap();
    owner.clock.set(owner.clock.now_ms() + 1_000);
    let (update, _, _) = get(&owner, &reader).await;
    assert_eq!((update.already_here, update.changed_here), (2, 0));
    assert_eq!(reader.read("b.txt"), b"two, changed");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_in_the_folder_is_not_overwritten() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    let reader = Reader::new();
    get(&owner, &reader).await;

    later();
    std::fs::write(reader.output.path().join("a.txt"), b"my notes").unwrap();
    std::fs::write(owner.folder.path().join("a.txt"), b"one, new version").unwrap();
    owner.clock.set(owner.clock.now_ms() + 1_000);
    let (report, _, changed) = get(&owner, &reader).await;

    assert_eq!(changed, vec!["a.txt"]);
    assert_eq!((report.already_here, report.changed_here), (1, 1));
    assert_eq!(reader.read("a.txt"), b"my notes");
    // Reported every time, until the user moves or deletes it.
    let (again, _, changed) = get(&owner, &reader).await;
    assert_eq!(changed, vec!["a.txt"]);
    assert_eq!(again.changed_here, 1);

    // Deleted here: the stick's version comes back.
    std::fs::remove_file(reader.output.path().join("a.txt")).unwrap();
    let (restored, _, _) = get(&owner, &reader).await;
    assert_eq!(restored.changed_here, 0);
    assert_eq!(reader.read("a.txt"), b"one, new version");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn files_that_left_the_stick_are_kept() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    let reader = Reader::new();
    get(&owner, &reader).await;

    std::fs::remove_file(owner.folder.path().join("b.txt")).unwrap();
    owner.clock.set(owner.clock.now_ms() + 1_000);
    let (report, gone, _) = get(&owner, &reader).await;

    assert_eq!(report.files, 1);
    assert_eq!(gone, Some(1));
    assert_eq!(reader.read("b.txt"), b"two");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_from_before_is_recognised_by_its_content() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    let first = Reader::new();
    get(&owner, &first).await;
    // Same folder, but a store that never saw it (e.g. an older version).
    std::fs::write(first.output.path().join("b.txt"), b"different").unwrap();
    let reader = Reader {
        store: Arc::new(MemoryStore::new()),
        blobs: tempfile::tempdir().unwrap(),
        ..first
    };

    let (report, _, changed) = get(&owner, &reader).await;
    assert_eq!((report.already_here, report.changed_here), (1, 1));
    assert_eq!(changed, vec!["b.txt"]);
    assert_eq!(report.received, 0, "nothing needed fetching");
    assert_eq!(reader.read("b.txt"), b"different");
    owner.publisher.shutdown().await;
}
