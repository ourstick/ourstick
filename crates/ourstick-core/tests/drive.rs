//! The app's stick is a drive: a folder of a set size, marked as the stick's.
//! An ejected drive (its folder gone, or an empty mount point) is never
//! published as an empty stick, and a drive that holds too much is not
//! published at all.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::Owner;
use ourstick_core::manifest::DRIVE_MARKER;
use ourstick_core::share::{folder_size, publish_folder, ShareError};
use ourstick_core::transport::Publisher;
use ourstick_core::watcher::{
    follow_folder, FolderEvents, Owner as Publishing, WatchEvent, WatchTiming,
};
use tokio::sync::mpsc;

async fn try_publish(owner: &Owner) -> Result<ourstick_core::share::Publication, ShareError> {
    publish_folder(
        &owner.publisher,
        owner.store.clone(),
        &owner.clock,
        &owner.signing_key,
        &owner.stick_key,
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drive_is_published_without_its_marker_in_the_manifest() {
    let owner = Owner::drive(1_000, &[("a.txt", b"hello")]).await;
    let publication = try_publish(&owner).await.unwrap();
    let paths: Vec<&str> = publication
        .manifest
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    assert_eq!(paths, vec!["a.txt"]);
    assert_eq!(folder_size(owner.folder.path()).unwrap(), 5);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ejected_drive_is_never_published_as_empty() {
    let owner = Owner::drive(1_000, &[("a.txt", b"hello")]).await;
    let first = try_publish(&owner).await.unwrap();

    // Ejected: the drive's contents are gone from the path, and an empty
    // folder (a mount point) is left there.
    let path = owner.folder.path().to_path_buf();
    let away = path.with_extension("detached");
    std::fs::rename(&path, &away).unwrap();
    std::fs::create_dir(&path).unwrap();
    owner.clock.set(5_000);
    let result = try_publish(&owner).await;
    assert!(
        matches!(result, Err(ShareError::DriveMissing { .. })),
        "{result:?}"
    );
    // And with no folder at all.
    std::fs::remove_dir(&path).unwrap();
    let result = try_publish(&owner).await;
    assert!(
        matches!(result, Err(ShareError::DriveMissing { .. })),
        "{result:?}"
    );

    // Plugged in again: the same version, not a new one.
    std::fs::rename(&away, &path).unwrap();
    let back = try_publish(&owner).await.unwrap();
    assert!(!back.new_version);
    assert_eq!(back.seq, first.seq);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn another_sticks_drive_is_not_this_stick() {
    let owner = Owner::drive(1_000, &[("a.txt", b"hello")]).await;
    std::fs::write(owner.folder.path().join(DRIVE_MARKER), "00".repeat(32)).unwrap();
    let result = try_publish(&owner).await;
    assert!(
        matches!(result, Err(ShareError::DriveMissing { .. })),
        "{result:?}"
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drive_that_holds_too_much_is_not_published() {
    let owner = Owner::drive(10, &[("a.txt", b"hello")]).await;
    try_publish(&owner).await.unwrap();

    std::fs::write(owner.folder.path().join("b.txt"), b"is too much").unwrap();
    owner.clock.set(5_000);
    let result = try_publish(&owner).await;
    assert!(
        matches!(result, Err(ShareError::StickFull { size: 10, used: 16 })),
        "{result:?}"
    );
    // System files do not count.
    std::fs::remove_file(owner.folder.path().join("b.txt")).unwrap();
    std::fs::write(owner.folder.path().join(".DS_Store"), vec![0; 6_000]).unwrap();
    assert!(try_publish(&owner).await.is_ok());
    owner.publisher.shutdown().await;
}

struct Manual(mpsc::UnboundedReceiver<()>);

impl FolderEvents for Manual {
    async fn changed(&mut self) -> Option<()> {
        self.0.recv().await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_says_ejected_once_and_picks_up_again() {
    let owner = Owner::drive(1_000, &[("a.txt", b"hello")]).await;
    let current = try_publish(&owner).await.unwrap().manifest;
    let (send, receive) = mpsc::unbounded_channel();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let mut on_event = move |event: WatchEvent<'_>| {
        record.lock().unwrap().push(match event {
            WatchEvent::Ejected => "ejected".to_owned(),
            WatchEvent::Published { .. } => "published".to_owned(),
            other => format!("{other:?}"),
        })
    };
    let publishing = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let timing = WatchTiming {
        quiet: Duration::from_millis(50),
        settle: Duration::from_millis(20),
        retry: Duration::from_millis(100),
    };
    let marker = owner.folder.path().join(DRIVE_MARKER);
    let mut events = Manual(receive);
    tokio::select! {
        () = follow_folder(&publishing, owner.folder.path(), &mut events, timing, current, &mut on_event) => {}
        () = async {
            std::fs::remove_file(&marker).unwrap();
            send.send(()).unwrap();
            // Several retries pass while ejected.
            tokio::time::sleep(Duration::from_millis(600)).await;
            ourstick_core::share::mark_drive(owner.folder.path(), &owner.signing_key.stick_id()).unwrap();
            owner.clock.set(9_000);
            std::fs::write(owner.folder.path().join("b.txt"), b"new").unwrap();
            send.send(()).unwrap();
            // Until the publish, which takes longer on a slow machine (a
            // Windows VM running every test at once needed over 500 ms).
            let published = || seen.lock().unwrap().iter().any(|event| event == "published");
            for _ in 0..100 {
                if published() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => {}
    }
    // A retry may land while the files are being written ("still changing"),
    // or publish before the last sign of change comes ("unchanged"); what
    // matters is one "ejected", then a publish.
    let seen: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|event| *event != "StillChanging" && *event != "Unchanged")
        .cloned()
        .collect();
    assert_eq!(seen, vec!["ejected", "published"]);
    owner.publisher.shutdown().await;
}
