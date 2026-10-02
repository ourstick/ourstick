//! ADR 0012 with two peers on this machine: what the user chose not to get
//! of a received stick is not fetched; anything else comes, also what the
//! sender adds later; un-ticking something that came leaves its files.

mod common;

use std::ops::ControlFlow;
use std::sync::Mutex;

use common::{keep_going, write_file, Owner, Reader};
use ourstick_core::download::DownloadEvent;
use ourstick_core::transport::Publisher;
use ourstick_core::{JoinedStick, Store};

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_not_chosen_does_not_come_and_everything_else_does() {
    let owner = Owner::new(&[
        ("Films/a.mkv", b"film"),
        ("Music/b.mp3", b"song"),
        ("note.txt", b"note"),
    ])
    .await;
    let first = owner.publish().await;
    let ticket = owner.ticket(&first);
    let reader = Reader::new();
    // Added, with "Films" un-ticked, before its first round.
    reader
        .store
        .add_joined_stick(&JoinedStick::new(ticket.stick_id, reader.output.path(), 1))
        .unwrap();
    reader
        .store
        .set_left_out(&ticket.stick_id, &names(&["Films"]))
        .unwrap();
    let told = Mutex::new(Vec::new());
    let report = reader
        .download(&ticket, &mut |event| {
            if let DownloadEvent::NotChosen { name, files, bytes } = event {
                told.lock().unwrap().push((name.to_owned(), files, bytes));
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
    assert!(!reader.exists("Films"));
    assert_eq!(reader.read("Music/b.mp3"), b"song");
    assert_eq!(reader.read("note.txt"), b"note");
    assert_eq!(report.not_chosen, 1);
    assert_eq!(told.into_inner().unwrap(), [("Films".to_owned(), 1, 4)]);

    // The sender adds a folder: it comes.
    write_file(owner.folder.path(), "Books/c.pdf", b"book");
    owner.clock.set(5_000);
    let second = owner.publish().await;
    reader
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Books/c.pdf"), b"book");
    assert!(!reader.exists("Films"));

    // Ticked again: it comes too.
    reader.store.set_left_out(&ticket.stick_id, &[]).unwrap();
    reader
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("Films/a.mkv"), b"film");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn un_ticking_what_came_leaves_its_files_and_stops_updating_them() {
    let owner = Owner::new(&[("Music/b.mp3", b"song"), ("note.txt", b"note")]).await;
    let first = owner.publish().await;
    let ticket = owner.ticket(&first);
    let reader = Reader::new();
    reader.download(&ticket, &mut keep_going).await.unwrap();
    assert_eq!(reader.read("Music/b.mp3"), b"song");

    reader
        .store
        .set_left_out(&ticket.stick_id, &names(&["Music"]))
        .unwrap();
    write_file(owner.folder.path(), "Music/b.mp3", b"remastered");
    write_file(owner.folder.path(), "note.txt", b"note 2");
    owner.clock.set(5_000);
    let second = owner.publish().await;
    let report = reader
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    // Its file stays as it was; the rest is kept up to date.
    assert_eq!(reader.read("Music/b.mp3"), b"song");
    assert_eq!(reader.read("note.txt"), b"note 2");
    assert_eq!(report.not_chosen, 1);
    owner.publisher.shutdown().await;
}
