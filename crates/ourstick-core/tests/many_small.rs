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
    // Every file told done before the stop is recorded (the one whose event
    // stopped it is in place, and found by content next time).
    for path in &done[..299] {
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
