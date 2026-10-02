//! ADR 0010 with two peers on this machine: a name this computer cannot
//! save under its own name is saved under the smallest change of it, every
//! other file comes, and later versions write to the same places.
//!
//! The names used here are refused on every system (`c:`, OurStick's own
//! names) or in the way of a file already here, so the tests run the same
//! everywhere; Windows' own rules are tested in `download::saved_names` and
//! `download::saved_paths`.

mod common;

use std::ops::ControlFlow;
use std::sync::Mutex;

use common::{keep_going, write_file, Owner, Reader};
use ourstick_core::download::DownloadEvent;
use ourstick_core::transport::Publisher;
use ourstick_core::Store;

/// The `SavedAs` events of a download, as (stick path, saved as, files).
fn saved_as_events(
    events: &Mutex<Vec<(String, String, usize)>>,
) -> impl FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send + '_ {
    move |event| {
        if let DownloadEvent::SavedAs {
            path,
            saved_as,
            files,
        } = event
        {
            events
                .lock()
                .unwrap()
                .push((path.to_owned(), saved_as.to_owned(), files));
        }
        ControlFlow::Continue(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn names_refused_here_are_saved_under_a_changed_name_and_everything_comes() {
    // A Windows owner cannot have `c:evil` (it names drive C:): elsewhere only.
    let mut files: Vec<(&str, &[u8])> =
        vec![("ok.txt", b"fine"), (".OURSTICK-PARTIAL/x", b"hidden")];
    if !cfg!(windows) {
        files.push(("c:evil", b"escape"));
    }
    let owner = Owner::new(&files).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    let events = Mutex::new(Vec::new());
    let report = reader
        .download(&ticket, &mut saved_as_events(&events))
        .await
        .unwrap();
    assert_eq!(reader.read("ok.txt"), b"fine");
    assert_eq!(reader.read("_.OURSTICK-PARTIAL/x"), b"hidden");
    assert!(!reader.exists(".OURSTICK-PARTIAL") || cfg!(any(windows, target_os = "macos")));
    let mut expected = vec![(
        ".OURSTICK-PARTIAL".to_owned(),
        "_.OURSTICK-PARTIAL".to_owned(),
        1,
    )];
    if !cfg!(windows) {
        assert_eq!(reader.read("c_evil"), b"escape");
        assert!(!reader.exists("c:evil"));
        expected.insert(0, ("c:evil".to_owned(), "c_evil".to_owned(), 1));
    }
    let mut seen = events.into_inner().unwrap();
    seen.sort();
    expected.sort();
    assert_eq!(seen, expected);
    assert_eq!(report.saved_as, expected.len());
    // The record is accepted; what was renamed is recorded so.
    assert!(reader
        .store
        .record_floor(&ticket.stick_id)
        .unwrap()
        .is_some());
    let renamed: Vec<String> = reader
        .store
        .saved_elsewhere(&ticket.stick_id)
        .unwrap()
        .into_iter()
        .map(|file| file.saved_as)
        .collect();
    assert_eq!(renamed.len(), expected.len());
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn later_versions_write_to_the_places_chosen_once() {
    let owner = Owner::new(&[(".OurStick-Partial/a.txt", b"one")]).await;
    let first = owner.publish().await;
    let reader = Reader::new();
    reader
        .download(&owner.ticket(&first), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("_.OurStick-Partial/a.txt"), b"one");

    // The file changes, and a file is added to the renamed folder.
    write_file(owner.folder.path(), ".OurStick-Partial/a.txt", b"two");
    write_file(owner.folder.path(), ".OurStick-Partial/b.txt", b"new");
    owner.clock.set(5_000);
    let second = owner.publish().await;
    reader
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("_.OurStick-Partial/a.txt"), b"two");
    assert_eq!(reader.read("_.OurStick-Partial/b.txt"), b"new");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_in_the_way_moves_the_folder_and_is_left_alone() {
    let owner = Owner::new(&[("trip/day1.jpg", b"sea")]).await;
    let publication = owner.publish().await;
    let reader = Reader::new();
    // The user has a file called `trip` in the folder.
    write_file(reader.output.path(), "trip", b"mine");
    let events = Mutex::new(Vec::new());
    reader
        .download(&owner.ticket(&publication), &mut saved_as_events(&events))
        .await
        .unwrap();
    assert_eq!(reader.read("trip"), b"mine");
    assert_eq!(reader.read("trip (2)/day1.jpg"), b"sea");
    assert_eq!(
        events.into_inner().unwrap(),
        [("trip".to_owned(), "trip (2)".to_owned(), 1)]
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_round_goes_on_without_renaming_again() {
    let owner = Owner::new(&[("c_/x.txt", b"x"), (".OurStick-Received", b"m")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    reader.download(&ticket, &mut keep_going).await.unwrap();
    assert_eq!(reader.read("_.OurStick-Received"), b"m");
    // Its row lost (as if stopped between placing and recording): the file
    // there holds its content, so it keeps its place.
    reader
        .store
        .forget_downloaded_file(&ticket.stick_id, ".OurStick-Received")
        .unwrap();
    let events = Mutex::new(Vec::new());
    reader
        .download(&ticket, &mut saved_as_events(&events))
        .await
        .unwrap();
    assert!(!reader.exists("_.OurStick-Received (2)"));
    assert_eq!(
        events.into_inner().unwrap(),
        [(
            ".OurStick-Received".to_owned(),
            "_.OurStick-Received".to_owned(),
            1
        )]
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_copy_the_user_deleted_comes_again_at_its_place() {
    let owner = Owner::new(&[(".OurStick-Partial", b"file")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    reader.download(&ticket, &mut keep_going).await.unwrap();
    std::fs::remove_file(reader.output.path().join("_.OurStick-Partial")).unwrap();
    owner.clock.set(5_000);
    let again = owner.publish_with(true).await;
    reader
        .download(&owner.ticket(&again), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("_.OurStick-Partial"), b"file");
    owner.publisher.shutdown().await;
}
