//! Many small files (ADR 0014): their records are written in groups, all of
//! them whatever ends the round, and a group lost to a power cut costs no
//! fetching, only a check of the files by content.

mod common;

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use common::{keep_going, noise, Owner, Reader};
use ourstick_core::download::{DownloadError, DownloadEvent};
use ourstick_core::transport::Publisher;
use ourstick_core::Store;

/// More than one group of records (256).
const FILES: usize = 400;

fn small_files() -> Vec<(String, Vec<u8>)> {
    (0..FILES)
        .map(|index| {
            let name = format!("{}/{index:03}.bin", index % 7);
            (name, noise(100 + index, index as u8))
        })
        .collect()
}

fn recorded(reader: &Reader, owner: &Owner) -> BTreeSet<String> {
    let stick_id = owner.signing_key.stick_id();
    reader
        .store
        .downloaded_files(&stick_id)
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_records_every_file_placed_also_short_of_a_group() {
    let files = small_files();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::new(&listed).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();

    // Stopped after 300 files: one group of 256 written, 44 waiting.
    let mut done = Vec::new();
    let stopped = reader
        .download(&ticket, &mut |event| {
            if let DownloadEvent::FileDone { path, .. } = event {
                done.push(path.to_owned());
                if done.len() == 300 {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        })
        .await;
    assert!(matches!(stopped, Err(DownloadError::Cancelled)));
    let kept = recorded(&reader, &owner);
    // Every file told done is recorded, the one whose telling stopped it
    // too (it was in place before it was told).
    for path in &done {
        assert!(kept.contains(path), "{path} is placed but not recorded");
    }
    for path in &kept {
        assert!(reader.exists(path), "{path} is recorded but not here");
    }

    // The rest comes, nothing twice: the file whose event stopped it is
    // found in place, so at most the 100 never placed are fetched.
    let mut fetched = 0;
    reader
        .download(&ticket, &mut |event| {
            if let DownloadEvent::FileStarted { .. } = event {
                fetched += 1;
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
    assert!(fetched <= FILES - 300, "{fetched} fetched again");
    assert_eq!(recorded(&reader, &owner).len(), FILES);
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes);
    }
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn records_lost_to_a_power_cut_cost_no_fetching() {
    let files = small_files();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::new(&listed).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    reader.download(&ticket, &mut keep_going).await.unwrap();

    // As if the last groups' commits never reached the disk: more than one
    // group of files in place, synced, and not recorded.
    let stick_id = owner.signing_key.stick_id();
    for (name, _) in &files[..300] {
        reader
            .store
            .forget_downloaded_file(&stick_id, name)
            .unwrap();
    }
    assert_eq!(recorded(&reader, &owner).len(), FILES - 300);

    let again = reader.download(&ticket, &mut keep_going).await.unwrap();
    assert_eq!(again.received, 0, "found equal by content, not fetched");
    assert_eq!(recorded(&reader, &owner).len(), FILES);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn twins_and_a_big_file_among_small_ones_all_come_whole() {
    // Small files, identical ones spread among them (one blob), and a file
    // too big to be placed beside others, in the middle.
    let twin = noise(3000, 200);
    let big = noise(5 << 20, 201);
    let mut files: Vec<(String, Vec<u8>)> = (0..60)
        .map(|index| (format!("{index:02}.bin"), noise(500 + index, index as u8)))
        .collect();
    for index in [3, 4, 17, 40, 59] {
        files[index].1 = twin.clone();
    }
    files[30].1 = big.clone();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::new(&listed).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();

    let mut copied = 0;
    reader
        .download(&ticket, &mut |event| {
            if let DownloadEvent::FileCopiedHere { .. } = event {
                copied += 1;
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
    // One twin fetched, the others copied from it once it was placed.
    assert_eq!(copied, 4);
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes, "{name}");
    }
    assert_eq!(recorded(&reader, &owner).len(), files.len());
    assert!(!reader.exists(".ourstick-partial"));
    owner.publisher.shutdown().await;
}
