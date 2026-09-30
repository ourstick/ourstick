//! The owner follows its folder: changes are published as new versions,
//! bursts once, nothing when nothing changed, and a failure does not stop it.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{keep_going, Owner, Reader};
use ourstick_core::share::PartialTiming;
use ourstick_core::transport::Publisher;
use ourstick_core::watcher::{
    follow_folder, follow_folder_reading_first, FolderEvents, NotifyEvents, Owner as Publishing,
    WatchEvent, WatchTiming,
};
use tokio::sync::mpsc;

const FAST: WatchTiming = WatchTiming {
    quiet: Duration::from_millis(100),
    settle: Duration::from_millis(50),
    retry: Duration::from_millis(300),
    partial: None,
};

/// Signs of change sent by the test.
struct Manual(mpsc::UnboundedReceiver<()>);

impl FolderEvents for Manual {
    async fn changed(&mut self) -> Option<()> {
        self.0.recv().await
    }
}

/// What the watcher reported, as short strings.
type Seen = Arc<Mutex<Vec<String>>>;

fn describe(event: WatchEvent<'_>) -> String {
    match event {
        WatchEvent::StillChanging => "still changing".to_owned(),
        WatchEvent::Unchanged => "unchanged".to_owned(),
        WatchEvent::Failed { .. } => "failed".to_owned(),
        WatchEvent::Ejected => "ejected".to_owned(),
        WatchEvent::Reading { done, total } => format!("reading {done}/{total}"),
        WatchEvent::Published {
            changes,
            publication,
        } => format!(
            "published {}{} +{:?} ~{:?} -{:?}",
            publication.seq,
            if publication.manifest.partial {
                " partial"
            } else {
                ""
            },
            changes.added,
            changes.changed,
            changes.removed
        ),
    }
}

/// Runs the watcher until `drive` returns, recording what it reports.
async fn watch_while(
    owner: &Owner,
    events: &mut impl FolderEvents,
    drive: impl std::future::Future<Output = ()>,
) -> Vec<String> {
    watch_with(owner, events, FAST, drive).await
}

/// [`watch_while`] with other timing.
async fn watch_with(
    owner: &Owner,
    events: &mut impl FolderEvents,
    timing: WatchTiming,
    drive: impl std::future::Future<Output = ()>,
) -> Vec<String> {
    let current = owner.publish().await.manifest;
    let publishing = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let seen: Seen = Arc::default();
    let record = seen.clone();
    // Progress is left out: every pass reports it.
    let mut on_event = move |event: WatchEvent<'_>| {
        if !matches!(event, WatchEvent::Reading { .. }) {
            record.lock().unwrap().push(describe(event));
        }
    };
    tokio::select! {
        () = follow_folder(&publishing, owner.folder.path(), events, timing, current, &mut on_event) => {}
        () = drive => {}
    }
    let seen = seen.lock().unwrap().clone();
    seen
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_changes_is_published_once() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    owner.clock.set(1_000);
    let (send, receive) = mpsc::unbounded_channel();
    let folder = owner.folder.path().to_path_buf();

    let seen = watch_while(&owner, &mut Manual(receive), async {
        owner.clock.set(2_000);
        std::fs::write(folder.join("b.txt"), b"two, changed").unwrap();
        send.send(()).unwrap();
        std::fs::write(folder.join("c.txt"), b"three").unwrap();
        send.send(()).unwrap();
        std::fs::remove_file(folder.join("a.txt")).unwrap();
        send.send(()).unwrap();
        pause(800).await;
    })
    .await;

    assert_eq!(
        seen,
        vec![r#"published 2000 +["c.txt"] ~["b.txt"] -["a.txt"]"#]
    );
    // A reader gets the new version.
    let latest = owner.publish().await;
    let reader = Reader::new();
    reader
        .download(&owner.ticket(&latest), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("b.txt"), b"two, changed");
    assert_eq!(reader.read("c.txt"), b"three");
    assert!(!reader.exists("a.txt"));
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_without_a_change_publishes_nothing_new() {
    let owner = Owner::new(&[("a.txt", b"one")]).await;
    owner.clock.set(1_000);
    let (send, receive) = mpsc::unbounded_channel();

    let seen = watch_while(&owner, &mut Manual(receive), async {
        owner.clock.set(2_000);
        send.send(()).unwrap();
        pause(500).await;
    })
    .await;

    assert_eq!(seen, vec!["unchanged"]);
    assert_eq!(owner.publish().await.seq, 1_000);
    owner.publisher.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_publish_is_retried_and_the_watch_goes_on() {
    use std::os::unix::fs::PermissionsExt;
    let owner = Owner::new(&[("a.txt", b"one")]).await;
    owner.clock.set(1_000);
    let (send, receive) = mpsc::unbounded_channel();
    let locked = owner.folder.path().join("locked.txt");

    let seen = watch_while(&owner, &mut Manual(receive), async {
        owner.clock.set(2_000);
        std::fs::write(&locked, b"secret").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        send.send(()).unwrap();
        pause(250).await;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        // No new sign: the retry timer publishes it.
        pause(800).await;
    })
    .await;

    assert_eq!(seen[0], "failed");
    assert_eq!(
        seen.last().unwrap(),
        r#"published 2000 +["locked.txt"] ~[] -[]"#
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operating_system_drives_it_end_to_end() {
    let owner = Owner::new(&[("a.txt", b"one")]).await;
    owner.clock.set(1_000);
    let mut events = NotifyEvents::start(owner.folder.path()).unwrap();
    let folder = owner.folder.path().to_path_buf();

    let seen = watch_while(&owner, &mut events, async {
        owner.clock.set(2_000);
        std::fs::create_dir(folder.join("new")).unwrap();
        std::fs::write(folder.join("new/b.txt"), b"two").unwrap();
        pause(1_500).await;
    })
    .await;

    // Publishing reads every file; those reads must not trigger more publishes.
    // Windows may report the new folder once more afterwards, which costs a
    // look that finds nothing changed.
    assert_eq!(seen[0], r#"published 2000 +["new/b.txt"] ~[] -[]"#);
    assert!(
        seen[1..].iter().all(|event| event == "unchanged"),
        "{seen:?}"
    );
    owner.publisher.shutdown().await;
}

/// ADR 0007: a pass over a big folder shows a new file early, in a partial
/// version that keeps every other file, then the complete version.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_pass_publishes_a_partial_version_first() {
    let files: Vec<(String, Vec<u8>)> = (0..600)
        .map(|i| (format!("f{i:04}.txt"), format!("file {i}").into_bytes()))
        .collect();
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, data)| (path.as_str(), data.as_slice()))
        .collect();
    let owner = Owner::new(&files).await;
    owner.clock.set(1_000);
    let (send, receive) = mpsc::unbounded_channel();
    let folder = owner.folder.path().to_path_buf();
    let timing = WatchTiming {
        partial: Some(PartialTiming {
            first_after: Duration::ZERO,
            every: Duration::ZERO,
        }),
        ..FAST
    };

    let seen = watch_with(&owner, &mut Manual(receive), timing, async {
        owner.clock.set(2_000);
        std::fs::write(folder.join("a-new.txt"), b"new").unwrap();
        send.send(()).unwrap();
        pause(3_000).await;
    })
    .await;

    assert_eq!(
        seen,
        vec![
            r#"published 2000 partial +["a-new.txt"] ~[] -[]"#,
            r#"published 2001 +[] ~[] -[]"#,
        ]
    );
    owner.publisher.shutdown().await;
}

/// After a start that only served the last version, the folder is read at
/// once, without waiting for a sign of change.
#[tokio::test(flavor = "multi_thread")]
async fn reading_first_publishes_what_changed_while_stopped() {
    let owner = Owner::new(&[("a.txt", b"one")]).await;
    let current = owner.publish().await.manifest;
    owner.clock.set(5_000);
    std::fs::write(owner.folder.path().join("b.txt"), b"two").unwrap();
    let publishing = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let (_send, receive) = mpsc::unbounded_channel();
    let mut events = Manual(receive);
    let mut seen = Vec::new();
    let mut on_event = |event: WatchEvent<'_>| {
        if !matches!(event, WatchEvent::Reading { .. }) {
            seen.push(describe(event));
        }
    };
    tokio::select! {
        () = follow_folder_reading_first(
            &publishing,
            owner.folder.path(),
            &mut events,
            FAST,
            current,
            &mut on_event,
        ) => {}
        () = pause(1_500) => {}
    }
    assert_eq!(seen, vec![r#"published 5000 +["b.txt"] ~[] -[]"#]);
    owner.publisher.shutdown().await;
}
