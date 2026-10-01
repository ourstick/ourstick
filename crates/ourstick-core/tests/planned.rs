//! ADR 0008: before fetching, a download tells what it will write into the
//! folder (what is there already left out), so the reader's program can
//! check that it fits; stopping it there leaves everything as it was.

mod common;

use std::ops::ControlFlow;

use common::{Owner, Reader};
use ourstick_core::download::{DownloadError, DownloadEvent};

/// Downloads, returning what `Planned` said, or stops there if `stop`.
async fn planned(owner: &Owner, reader: &Reader, stop: bool) -> (Option<u64>, bool) {
    let publication = owner.publish().await;
    let mut seen = None;
    let result = reader
        .download(&owner.ticket(&publication), &mut |event| {
            if let DownloadEvent::Planned { bytes } = event {
                seen = Some(bytes);
                if stop {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        })
        .await;
    let cancelled = matches!(result, Err(DownloadError::Cancelled));
    (seen, cancelled)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_tells_what_it_will_write_before_fetching() {
    let owner = Owner::new(&[("a.txt", b"hello"), ("b/c.bin", &[7; 3_000])]).await;
    let reader = Reader::new();

    // Stopped where it tells: nothing written.
    assert_eq!(planned(&owner, &reader, true).await, (Some(3_005), true));
    assert!(!reader.exists("a.txt"));

    assert_eq!(planned(&owner, &reader, false).await, (Some(3_005), false));
    // Only what changed is written again.
    owner.clock.set(5_000);
    std::fs::write(owner.folder.path().join("a.txt"), b"hello again").unwrap();
    assert_eq!(planned(&owner, &reader, false).await, (Some(11), false));
    assert_eq!(reader.read("a.txt"), b"hello again");
}
