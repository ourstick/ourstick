//! ADR 0008: the owner follows every root of its stick. A change in a
//! linked folder is published; one linked while watching is seen when the
//! app nudges; one that goes and comes back is looked at again until it is
//! back, with what changed meanwhile.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{write_file, Owner};
use ourstick_core::linked::plan_link;
use ourstick_core::watcher::{
    follow_folder, NotifyEvents, Owner as Publishing, WatchEvent, WatchTiming,
};
use ourstick_core::Store;

const FAST: WatchTiming = WatchTiming {
    quiet: Duration::from_millis(100),
    settle: Duration::from_millis(50),
    retry: Duration::from_millis(300),
    partial: None,
};

fn link(owner: &Owner, path: &Path) {
    let stick_id = owner.signing_key.stick_id();
    let entry = plan_link(
        &ourstick_core::sender_files::LocalFiles,
        owner.store.as_ref(),
        &stick_id,
        owner.cache.path(),
        &[],
        path,
        1,
    )
    .unwrap();
    owner.store.add_linked_entry(&stick_id, &entry).unwrap();
}

/// The paths each published version added, while `drive` runs.
async fn added_while(
    owner: &Owner,
    events: &mut NotifyEvents,
    drive: impl std::future::Future<Output = ()>,
) -> Vec<Vec<String>> {
    let current = owner.publish().await.manifest;
    let publishing = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        files: std::sync::Arc::new(ourstick_core::sender_files::LocalFiles),
        disks: common::disks(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let added: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = added.clone();
    let mut on_event = move |event: WatchEvent<'_>| {
        if let WatchEvent::Published { changes, .. } = event {
            record.lock().unwrap().push(changes.added.clone());
        }
    };
    tokio::select! {
        () = follow_folder(&publishing, events, FAST, current, &mut on_event) => {}
        () = drive => {}
    }
    let added = added.lock().unwrap().clone();
    added
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_in_a_linked_folder_is_published() {
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    write_file(&films, "x.mkv", b"x");
    let owner = Owner::new(&[("a.txt", b"a")]).await;
    link(&owner, &films);
    let mut events = NotifyEvents::start(owner.folder.path()).unwrap();
    let added = added_while(&owner, &mut events, async {
        pause(300).await;
        write_file(&films, "new/y.mkv", b"y");
        pause(1_500).await;
    })
    .await;
    assert_eq!(added, [vec!["Films/new/y.mkv".to_owned()]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_linked_while_watching_is_seen_when_nudged() {
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    write_file(&films, "x.mkv", b"x");
    let owner = Owner::new(&[("a.txt", b"a")]).await;
    let mut events = NotifyEvents::start(owner.folder.path()).unwrap();
    let nudge = events.nudge();
    let added = added_while(&owner, &mut events, async {
        pause(300).await;
        link(&owner, &films);
        nudge.now();
        pause(1_000).await;
        // Watched from then on.
        write_file(&films, "later.mkv", b"later");
        pause(1_500).await;
    })
    .await;
    assert_eq!(
        added,
        [
            vec!["Films/x.mkv".to_owned()],
            vec!["Films/later.mkv".to_owned()]
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_folder_that_comes_back_is_read_again_by_itself() {
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    let away = elsewhere.path().join("away");
    write_file(&films, "x.mkv", b"x");
    let owner = Owner::new(&[("a.txt", b"a")]).await;
    link(&owner, &films);
    let mut events = NotifyEvents::start(owner.folder.path()).unwrap();
    let added = added_while(&owner, &mut events, async {
        pause(300).await;
        std::fs::rename(&films, &away).unwrap();
        pause(800).await;
        // Changed while away, then back: no sign from the system needed.
        write_file(&away, "while-away.mkv", b"w");
        std::fs::rename(&away, &films).unwrap();
        pause(2_000).await;
    })
    .await;
    assert_eq!(added, [vec!["Films/while-away.mkv".to_owned()]]);
}
