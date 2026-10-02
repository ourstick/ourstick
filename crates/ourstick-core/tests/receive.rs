//! The receiver window's loop (step 7b): a stick is received into its marked
//! folder, kept up to date as the owner publishes, waited for when the owner
//! is away or the folder is not there, and stopped at once by a cancel.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::memory::{publish_versions, Link, MemoryFetcher, Versions};
use ourstick_core::clock::FixedClock;
use ourstick_core::download::{
    mark_received_folder, receive, Cancel, DownloadEvent, ReceiveEvent, Receiving, RelayPolicy,
    Waiting,
};
use ourstick_core::locks::download_lock;
use ourstick_core::space::{Disks, DownloadRoom};
use ourstick_core::{MemoryStore, StickKind, Store};

/// Every path on one disk with plenty of room: the tests never depend on
/// the machine's own disks.
struct RoomyDisk;

impl Disks for RoomyDisk {
    fn key(&self, _path: &std::path::Path) -> std::io::Result<String> {
        Ok("d".to_owned())
    }
    fn free(&self, _path: &std::path::Path) -> std::io::Result<u64> {
        Ok(500 << 30)
    }
    fn total(&self, _path: &std::path::Path) -> std::io::Result<u64> {
        Ok(1000 << 30)
    }
}

/// What a receive told, as plain values.
#[derive(Debug, Clone, PartialEq)]
enum Told {
    UpToDate(u64),
    Waiting(Waiting),
    File(String),
}

/// Runs a receive of `stick` into `folder` until `until` says it has seen
/// enough (then cancels), doing `then` (with the receiver's store) the first
/// time `trigger` is told.
async fn run_receive(
    stick: &Versions,
    folder: &std::path::Path,
    until: impl Fn(&[Told]) -> bool + Send + Sync + 'static,
    trigger: Told,
    then: impl Fn(&Arc<dyn Store>) + Send + Sync + 'static,
) -> Vec<Told> {
    let data = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let store_then = store.clone();
    let _lock = download_lock(data.path(), &stick.ticket.stick_id).unwrap();
    let room = DownloadRoom::with_disks(
        Box::new(RoomyDisk),
        store.clone(),
        data.path().to_path_buf(),
        stick.ticket.stick_id,
        folder.to_path_buf(),
    );
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    );
    let clock = FixedClock::new(5_000);
    let policy = RelayPolicy::default();
    let receiving = Receiving {
        kind: StickKind::App,
        fetcher: &fetcher,
        store: store.clone(),
        clock: &clock,
        ticket: &stick.ticket,
        folder,
        policy: &policy,
        room: &room,
        follow_every: Duration::from_millis(50),
    };
    let cancel = Cancel::new();
    let told = Arc::new(Mutex::new(Vec::new()));
    let mut triggered = false;
    let mut on_event = {
        let told = told.clone();
        let cancel = cancel.clone();
        move |event: ReceiveEvent<'_>| {
            let item = match event {
                ReceiveEvent::UpToDate { seq } => Told::UpToDate(seq),
                ReceiveEvent::Waiting { why, .. } => Told::Waiting(why),
                ReceiveEvent::Download(DownloadEvent::FileDone { path, .. }) => {
                    Told::File(path.to_owned())
                }
                ReceiveEvent::Download(_) => return,
            };
            let mut told = told.lock().unwrap();
            if !triggered && item == trigger {
                triggered = true;
                then(&store_then);
            }
            told.push(item);
            if until(&told) {
                cancel.cancel();
            }
        }
    };
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        receive(&receiving, &cancel, &mut on_event),
    )
    .await
    .expect("the receive stops when cancelled");
    result.unwrap();
    let told = told.lock().unwrap().clone();
    told
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stick_is_received_and_kept_up_to_date_until_cancelled() {
    let stick = Arc::new(
        publish_versions(&[
            &[("a.txt", b"one")],
            &[("a.txt", b"one"), ("b.txt", b"two")],
        ])
        .await,
    );
    let folder = tempfile::tempdir().unwrap();
    mark_received_folder(folder.path(), &stick.ticket.stick_id).unwrap();
    let publisher = stick.clone();
    let told = run_receive(
        &stick,
        folder.path(),
        |told| told.contains(&Told::UpToDate(2000)),
        Told::UpToDate(1000),
        move |_| publisher.publish(1),
    )
    .await;
    assert_eq!(std::fs::read(folder.path().join("a.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(folder.path().join("b.txt")).unwrap(), b"two");
    let up_to_date: Vec<_> = told
        .iter()
        .filter(|told| matches!(told, Told::UpToDate(_)))
        .collect();
    assert_eq!(up_to_date, [&Told::UpToDate(1000), &Told::UpToDate(2000)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_without_its_mark_is_waited_for_and_never_filled() {
    let stick = publish_versions(&[&[("a.txt", b"one")]]).await;
    let folder = tempfile::tempdir().unwrap();
    let told = run_receive(
        &stick,
        folder.path(),
        |told| told.contains(&Told::Waiting(Waiting::Folder)),
        Told::UpToDate(0),
        |_| {},
    )
    .await;
    assert_eq!(told, [Told::Waiting(Waiting::Folder)]);
    assert!(!folder.path().join("a.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_who_is_away_is_waited_for_and_says_why() {
    let stick = publish_versions(&[&[("a.txt", b"one")]]).await;
    stick.withdraw();
    let folder = tempfile::tempdir().unwrap();
    mark_received_folder(folder.path(), &stick.ticket.stick_id).unwrap();
    let told = run_receive(
        &stick,
        folder.path(),
        |told| !told.is_empty(),
        Told::UpToDate(0),
        |_| {},
    )
    .await;
    assert!(
        matches!(
            told[0],
            Told::Waiting(Waiting::Sender(_) | Waiting::NoLongerShared)
        ),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preview_groups_by_top_level_and_leaves_out_what_is_already_here() {
    use ourstick_core::download::{preview, TopEntry};
    let stick = publish_versions(&[&[
        ("photos/a.jpg", b"aa"),
        ("photos/b.jpg", b"bbb"),
        ("note.txt", b"note"),
    ]])
    .await;
    let fetcher = MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    );
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let folder = tempfile::tempdir().unwrap();
    let new_folder = folder.path().join("not yet");
    let chosen = new_folder.clone();
    let seen = preview(&fetcher, store.clone(), &stick.ticket, move |_| chosen)
        .await
        .unwrap();
    assert_eq!(seen.folder, new_folder);
    assert_eq!(
        seen.entries,
        [
            TopEntry {
                name: "note.txt".to_owned(),
                is_dir: false,
                files: 1,
                bytes: 4,
                to_write: 4,
            },
            TopEntry {
                name: "photos".to_owned(),
                is_dir: true,
                files: 2,
                bytes: 5,
                to_write: 5,
            },
        ]
    );
    assert_eq!((seen.total, seen.to_write), (9, 9));
    assert!(!seen.partial);
    assert!(!new_folder.exists(), "a preview writes nothing");
    // The same note is here already: only the photos are to come.
    std::fs::write(folder.path().join("note.txt"), b"note").unwrap();
    let here = folder.path().to_path_buf();
    let seen = preview(&fetcher, store, &stick.ticket, move |_| here)
        .await
        .unwrap();
    assert_eq!(seen.to_write, 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_who_goes_away_while_up_to_date_is_told_and_back_again() {
    let stick = Arc::new(publish_versions(&[&[("a.txt", b"one")]]).await);
    let folder = tempfile::tempdir().unwrap();
    mark_received_folder(folder.path(), &stick.ticket.stick_id).unwrap();
    let away = stick.clone();
    let told = run_receive(
        &stick,
        folder.path(),
        |told| {
            told.iter()
                .filter(|told| matches!(told, Told::UpToDate(_)))
                .count()
                >= 2
        },
        Told::UpToDate(1000),
        move |_| {
            // Away, then back a moment later.
            let back = away.clone();
            let served = away.served.lock().unwrap().clone();
            away.withdraw();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                *back.served.lock().unwrap() = served;
            });
        },
    )
    .await;
    let away_at = told
        .iter()
        .position(|told| {
            matches!(
                told,
                Told::Waiting(Waiting::Sender(_) | Waiting::NoLongerShared)
            )
        })
        .expect("the owner going away is told");
    assert!(told[away_at..].contains(&Told::UpToDate(1000)), "{told:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_in_the_middle_of_a_download_stops_it_soon() {
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|index| (format!("f{index:03}.bin"), vec![index as u8; 4096]))
        .collect();
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, data)| (path.as_str(), data.as_slice()))
        .collect();
    let stick = publish_versions(&[&files]).await;
    let folder = tempfile::tempdir().unwrap();
    mark_received_folder(folder.path(), &stick.ticket.stick_id).unwrap();
    let started = std::time::Instant::now();
    let told = run_receive(
        &stick,
        folder.path(),
        |told| told.iter().any(|told| matches!(told, Told::File(_))),
        Told::UpToDate(0),
        |_| {},
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(!told.contains(&Told::UpToDate(1000)), "{told:?}");
    // What landed is whole: every file there is one the stick has.
    for entry in std::fs::read_dir(folder.path()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if name.starts_with('f') {
            assert_eq!(
                std::fs::read(folder.path().join(&name)).unwrap().len(),
                4096
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_window_records_a_stick_before_its_first_round() {
    use ourstick_core::download::take_stick;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let stick_id = ourstick_core::StickSigningKey::from_bytes(&[5; 32]).stick_id();
    let folder = std::path::Path::new("/d/Holiday");
    take_stick(&store, stick_id, folder, StickKind::App, 7)
        .await
        .unwrap();
    let joined = store.joined_stick(&stick_id).unwrap().unwrap();
    assert_eq!(
        (joined.kind, joined.output_dir.as_path()),
        (StickKind::App, folder)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choice_saved_while_it_follows_counts_at_the_next_version() {
    let stick = Arc::new(
        publish_versions(&[
            &[("Films/a.mkv", b"film"), ("b.txt", b"two")],
            &[
                ("Films/a.mkv", b"film"),
                ("Films/c.mkv", b"more"),
                ("b.txt", b"two"),
                ("d.txt", b"new"),
            ],
        ])
        .await,
    );
    let folder = tempfile::tempdir().unwrap();
    mark_received_folder(folder.path(), &stick.ticket.stick_id).unwrap();
    let publisher = stick.clone();
    let stick_id = stick.ticket.stick_id;
    run_receive(
        &stick,
        folder.path(),
        |told| told.contains(&Told::UpToDate(2000)),
        Told::UpToDate(1000),
        move |store| {
            // Un-ticked while it is up to date and following.
            store
                .set_left_out(&stick_id, &["Films".to_owned()])
                .unwrap();
            publisher.publish(1);
        },
    )
    .await;
    assert_eq!(std::fs::read(folder.path().join("d.txt")).unwrap(), b"new");
    // What came before stays; what the sender added under it does not come.
    assert!(folder.path().join("Films/a.mkv").exists());
    assert!(!folder.path().join("Films/c.mkv").exists());
}
