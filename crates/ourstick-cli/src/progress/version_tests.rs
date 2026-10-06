use super::*;

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// Found by hand: the next version of a growing catalogue tells every file
/// again (those received as already here); they were counted twice, and
/// the line said "17.5 GB of 12.6 GB".
#[test]
fn a_new_version_counts_its_files_once() {
    let mut progress = Progress::default();
    let manifest = |bytes, partial| DownloadEvent::Manifest {
        seq: 1,
        files: 1,
        bytes,
        unchanged: false,
        partial,
    };
    progress.on_event(manifest(1_000_000_000, true), secs(0));
    progress.on_event(
        DownloadEvent::FileReceived {
            path: "a.mkv",
            size: 1_000_000_000,
            already_local: 0,
            received: 1_000_000_000,
        },
        secs(10),
    );
    progress.on_event(manifest(2_000_000_000, false), secs(20));
    progress.on_event(
        DownloadEvent::FileAlreadyHere {
            path: "a.mkv",
            size: 1_000_000_000,
        },
        secs(20),
    );
    progress.on_event(
        DownloadEvent::FileStarted {
            path: "b.mkv",
            size: 1_000_000_000,
            already_local: 0,
        },
        secs(20),
    );
    let line = progress.on_event(
        DownloadEvent::Progress {
            path: "b.mkv",
            received: 500_000_000,
        },
        secs(30),
    );
    assert_eq!(line.len(), 1);
    assert!(
        line[0].starts_with("[00:30] 1.5 GB of 2.0 GB (75%)"),
        "{line:?}"
    );
}
