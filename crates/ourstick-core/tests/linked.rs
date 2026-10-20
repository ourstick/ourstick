//! ADR 0008, two peers: folders and files linked to a stick are shared from
//! where they are; one that goes keeps its last version offered (readers
//! wait, nothing fails), and it is served again once back.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{fetch_when_ready, keep_going, write_file, Owner, Reader};
use ourstick_core::linked::plan_link;
use ourstick_core::transport::{Fetcher, PeerConnection, TransportError, GO};
use ourstick_core::Store;

/// Links `path` to the owner's stick.
fn link(owner: &Owner, path: &Path) {
    let stick_id = owner.signing_key.stick_id();
    let entry = plan_link(
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

#[tokio::test(flavor = "multi_thread")]
async fn linked_entries_are_shared_from_where_they_are() {
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    write_file(&films, "x.mkv", b"x");
    write_file(&films, "2024/y.mkv", b"y");
    let notes = elsewhere.path().join("notes.txt");
    std::fs::write(&notes, b"notes").unwrap();

    let owner = Owner::new(&[("a.txt", b"a")]).await;
    link(&owner, &films);
    link(&owner, &notes);
    let publication = owner.publish().await;
    let paths: Vec<&str> = publication
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["Films/2024/y.mkv", "Films/x.mkv", "a.txt", "notes.txt"]
    );

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Films/2024/y.mkv"), b"y");
    assert_eq!(reader.read("notes.txt"), b"notes");
    assert_eq!(reader.read("a.txt"), b"a");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_folder_that_goes_keeps_its_version_and_readers_wait_for_it() {
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    write_file(&films, "x.mkv", &common::noise(200_000, 3));
    let owner = Owner::new(&[("a.txt", b"a")]).await;
    link(&owner, &films);
    let first = owner.publish().await;

    // Its disk unplugged: the same version, not one without it.
    let away = elsewhere.path().join("away");
    std::fs::rename(&films, &away).unwrap();
    let again = owner.publish().await;
    assert!(!again.new_version);
    assert_eq!(again.manifest, first.manifest);

    // A reader waits ("not ready"), it is never told the file failed.
    let hash = first
        .manifest
        .entries
        .iter()
        .find(|entry| entry.path == "Films/x.mkv")
        .unwrap()
        .hash;
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&owner.ticket(&again)).await.unwrap();
    for _ in 0..10 {
        let result = connection.fetch_blob(&hash, &mut |_| GO).await;
        assert!(
            matches!(result, Err(TransportError::NotReady)),
            "{result:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Back: served.
    std::fs::rename(&away, &films).unwrap();
    fetch_when_ready(&connection, &hash, &mut |_| GO)
        .await
        .unwrap();
    fetcher.shutdown(true).await.unwrap();
}

/// ADR 0028 §2, amendment of 2026-10-20: a stick made from one folder sends
/// its contents at the top, and what is added later goes next to them; the
/// reader gets no folder inside a folder of the same name.
#[tokio::test(flavor = "multi_thread")]
async fn the_one_folder_a_stick_was_made_from_is_its_top() {
    let elsewhere = tempfile::tempdir().unwrap();
    let holiday = elsewhere.path().join("Holiday 2026");
    write_file(&holiday, "Beach/x.jpg", b"x");
    write_file(&holiday, "note.txt", b"n");
    let notes = elsewhere.path().join("notes.txt");
    std::fs::write(&notes, b"notes").unwrap();

    let owner = Owner::sending().await;
    let stick_id = owner.signing_key.stick_id();
    let mut planned = vec![plan_link(
        owner.store.as_ref(),
        &stick_id,
        owner.cache.path(),
        &[],
        &holiday,
        1,
    )
    .unwrap()];
    ourstick_core::linked::one_folder_at_top(&mut planned);
    assert!(planned[0].at_top);
    owner
        .store
        .add_linked_entry(&stick_id, &planned[0])
        .unwrap();
    link(&owner, &notes);

    let publication = owner.publish().await;
    let paths: Vec<&str> = publication
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths, ["Beach/x.jpg", "note.txt", "notes.txt"]);

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Beach/x.jpg"), b"x");
    assert_eq!(reader.read("note.txt"), b"n");
    assert_eq!(reader.read("notes.txt"), b"notes");
}
