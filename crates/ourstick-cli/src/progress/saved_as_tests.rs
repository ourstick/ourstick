use super::*;
use ourstick_core::transport::PathUsage;
use std::time::Duration;

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn names_saved_under_another_are_told_escaped_and_counted() {
    let mut progress = Progress::default();
    assert_eq!(
        progress.on_event(
            DownloadEvent::SavedAs {
                path: "a?.jpg",
                saved_as: "a_.jpg",
                files: 1,
            },
            secs(1),
        ),
        vec!["[00:01] \u{201c}a?.jpg\u{201d} is saved here as \u{201c}a_.jpg\u{201d} (its name cannot be saved here as it is)."]
    );
    let folder = progress.on_event(
        DownloadEvent::SavedAs {
            path: "Trip\u{1b}?",
            saved_as: "Trip\u{1b}_",
            files: 412,
        },
        secs(1),
    );
    assert!(folder[0].contains("(a folder of 412 files)"), "{folder:?}");
    assert!(!folder[0].contains('\u{1b}'), "{folder:?}");
    let report = DownloadReport {
        seq: 1,
        files: 413,
        bytes: 10,
        received: 10,
        already_here: 0,
        changed_here: 0,
        saved_as: 413,
        left_out: 0,
        usage: PathUsage::default(),
    };
    let summary = Progress::summary(&report, "/out", secs(3), secs(3));
    assert_eq!(
        summary.last().unwrap(),
        "        Saved under slightly different names: 413 files (listed above)."
    );
}

#[test]
fn a_file_left_out_is_told_and_counted() {
    let mut progress = Progress::default();
    assert_eq!(
        progress.on_event(
            DownloadEvent::FileLeftOut {
                path: "Trip/a\u{1b}.jpg",
                size: 3,
            },
            secs(1),
        ),
        vec!["[00:01]   Trip/a\\u{1b}.jpg: cannot be saved in this folder (something here is in its way), left out"]
    );
    let report = DownloadReport {
        seq: 1,
        files: 2,
        bytes: 10,
        received: 10,
        already_here: 0,
        changed_here: 0,
        saved_as: 0,
        left_out: 1,
        usage: PathUsage::default(),
    };
    let summary = Progress::summary(&report, "/out", secs(3), secs(3));
    assert_eq!(
        summary.last().unwrap(),
        "        Not downloaded: 1 file could not be saved here."
    );
}
