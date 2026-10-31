//! ADR 0029: a stick whose files are read only through `SenderFiles`, none
//! of them on disk (as a phone's chosen files), reaches a reader whole, and
//! a change reaches it with the next version.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{keep_going, noise, Owner, Reader};
use ourstick_core::linked::plan_link;
use ourstick_core::sender_files::MemoryFiles;
use ourstick_core::Store;

fn link(owner: &Owner, path: &Path) {
    let stick_id = owner.signing_key.stick_id();
    let entry = plan_link(
        owner.files.as_ref(),
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
async fn files_read_only_through_the_trait_reach_a_reader() {
    let somewhere = tempfile::tempdir().unwrap();
    let phone = Arc::new(MemoryFiles::new(somewhere.path().join("chosen")));
    let big = noise(300_000, 1);
    phone.write("Camera/IMG_1.jpg", &big);
    phone.write("Camera/2024/IMG_2.jpg", &noise(50_000, 2));
    phone.write("notes.txt", b"notes");
    let owner = Owner::sending_through(phone.clone()).await;
    link(&owner, &phone.path("Camera"));
    link(&owner, &phone.path("notes.txt"));

    let publication = owner.publish().await;
    let paths: Vec<&str> = publication
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["Camera/2024/IMG_2.jpg", "Camera/IMG_1.jpg", "notes.txt"]
    );
    assert!(
        !somewhere.path().join("chosen").exists(),
        "nothing was on disk"
    );

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Camera/IMG_1.jpg"), big);
    assert_eq!(reader.read("Camera/2024/IMG_2.jpg"), noise(50_000, 2));
    assert_eq!(reader.read("notes.txt"), b"notes");

    // Changed where it is: the next version has it.
    phone.write("Camera/IMG_1.jpg", b"edited");
    phone.remove("Camera/2024/IMG_2.jpg");
    let next = owner.publish().await;
    assert!(next.seq > publication.seq);
    reader
        .download(&owner.ticket(&next), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Camera/IMG_1.jpg"), b"edited");
    let listed: Vec<&str> = next
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(listed, ["Camera/IMG_1.jpg", "notes.txt"]);
}
