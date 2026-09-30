use super::*;
use ourstick_core::transport::PathUsage;

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn clock_is_short_and_grows_hours() {
    assert_eq!(clock(secs(0)), "00:00");
    assert_eq!(clock(secs(151)), "02:31");
    assert_eq!(clock(secs(3_725)), "1:02:05");
}

#[test]
fn rate_is_per_second() {
    assert_eq!(rate(10_000_000, secs(2)), "5.0 MB/s");
    assert_eq!(rate(5, Duration::ZERO), "–");
}

#[test]
fn every_line_has_the_time_and_progress_is_throttled() {
    let mut progress = Progress::default();
    let connected = progress.on_event(
        DownloadEvent::Connected {
            path: PathKind::Relay,
        },
        secs(1),
    );
    assert_eq!(connected.len(), 1);
    assert!(connected[0].starts_with("[00:01] Connected: through a relay"));

    progress.on_event(
        DownloadEvent::Manifest {
            seq: 1,
            files: 1,
            bytes: 1_000_000_000,
            unchanged: false,
        },
        secs(1),
    );
    progress.on_event(
        DownloadEvent::FileStarted {
            path: "movie.mkv",
            size: 1_000_000_000,
            already_local: 0,
        },
        secs(1),
    );
    let progress_event = |received| DownloadEvent::Progress {
        path: "movie.mkv",
        received,
    };
    // First line once 3 s passed since the start.
    assert!(progress.on_event(progress_event(1_000), secs(2)).is_empty());
    let line = progress.on_event(progress_event(300_000_000), secs(4));
    assert_eq!(
        line,
        vec!["[00:04] 300.0 MB of 1.0 GB (30%), 75.0 MB/s, relay"]
    );
    // Nothing again until 3 s later.
    assert!(progress
        .on_event(progress_event(400_000_000), secs(6))
        .is_empty());

    let changed = progress.on_event(
        DownloadEvent::PathChanged {
            path: PathKind::Direct,
        },
        secs(6),
    );
    assert_eq!(changed, vec!["[00:06] Now: direct connection."]);
    let line = progress.on_event(progress_event(700_000_000), secs(7));
    assert_eq!(
        line,
        vec!["[00:07] 700.0 MB of 1.0 GB (70%), 133.3 MB/s, direct"]
    );
}

#[test]
fn the_relay_limit_is_said_once_and_stays_in_view_until_direct() {
    let mut progress = Progress::default();
    progress.on_event(
        DownloadEvent::Connected {
            path: PathKind::Relay,
        },
        secs(0),
    );
    progress.on_event(
        DownloadEvent::Manifest {
            seq: 1,
            files: 1,
            bytes: 100_000_000,
            unchanged: false,
        },
        secs(0),
    );
    assert_eq!(
        progress.on_event(
            DownloadEvent::RelayLimited {
                bits_per_second: 10_000_000
            },
            secs(4)
        ),
        vec![
            "[00:04] No direct connection found; continuing through the relay, \
             up to 10 Mbps on the free relays. A direct connection is still looked for."
        ]
    );
    let line = progress.on_event(
        DownloadEvent::Progress {
            path: "movie.mkv",
            received: 5_000_000,
        },
        secs(8),
    );
    assert!(line[0].ends_with(", relay, up to 10 Mbps"), "{line:?}");
    progress.on_event(
        DownloadEvent::PathChanged {
            path: PathKind::Direct,
        },
        secs(9),
    );
    let line = progress.on_event(
        DownloadEvent::Progress {
            path: "movie.mkv",
            received: 50_000_000,
        },
        secs(12),
    );
    assert!(line[0].ends_with(", direct"), "{line:?}");
    assert_eq!(mbps(2_500_000), "2.5 Mbps");
}

#[test]
fn resumed_file_counts_what_is_here_but_speed_only_the_network() {
    let mut progress = Progress::default();
    progress.on_event(
        DownloadEvent::Manifest {
            seq: 1,
            files: 2,
            bytes: 6_000_000_000,
            unchanged: true,
        },
        secs(0),
    );
    // A small file that was already complete: finished at once, nothing received.
    progress.on_event(
        DownloadEvent::FileStarted {
            path: "extra.mkv",
            size: 200_000_000,
            already_local: 200_000_000,
        },
        secs(0),
    );
    progress.on_event(
        DownloadEvent::FileDone {
            path: "extra.mkv",
            already_local: 200_000_000,
            received: 0,
        },
        secs(1),
    );
    // The big file had 1.2 GB from the interrupted run.
    progress.on_event(
        DownloadEvent::FileStarted {
            path: "movie.mkv",
            size: 5_800_000_000,
            already_local: 1_200_000_000,
        },
        secs(1),
    );
    let line = progress.on_event(
        DownloadEvent::Progress {
            path: "movie.mkv",
            received: 21_000_000,
        },
        secs(3),
    );
    assert_eq!(line, vec!["[00:03] 1.4 GB of 6.0 GB (23%), 7.0 MB/s"]);
}

#[test]
fn file_lines_are_escaped_and_show_resume() {
    let mut progress = Progress::default();
    let done = progress.on_event(
        DownloadEvent::FileDone {
            path: "\u{1b}[2Jmovie.mkv",
            already_local: 2_000_000,
            received: 3_000_000,
        },
        secs(65),
    );
    assert_eq!(
        done,
        vec!["[01:05]   \\u{1b}[2Jmovie.mkv (3.0 MB received, resumed after 2.0 MB)"]
    );
}

#[test]
fn summary_says_how_long_and_which_path() {
    let report = DownloadReport {
        seq: 1,
        files: 7,
        bytes: 5_900_000_000,
        received: 5_900_000_000,
        already_here: 0,
        changed_here: 0,
        usage: PathUsage {
            direct: 5_000_000_000,
            relay: 950_000_000,
            direct_ipv6: 4_000_000_000,
        },
    };
    let lines = Progress::summary(&report, "/Users/a/received", secs(252), secs(252));
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0],
        "[04:12] Done in 04:12: 7 files in /Users/a/received (5.9 GB received, average 23.4 MB/s)."
    );
    assert_eq!(
        lines[1],
        "        Received directly: 5.0 GB (4.0 GB over IPv6), through a relay: 950.0 MB."
    );
}

#[test]
fn files_not_downloaded_are_accounted_for() {
    let mut progress = Progress::default();
    progress.on_event(
        DownloadEvent::Manifest {
            seq: 1,
            files: 3,
            bytes: 300,
            unchanged: false,
        },
        secs(0),
    );
    assert!(progress
        .on_event(
            DownloadEvent::FileAlreadyHere {
                path: "a",
                size: 100
            },
            secs(1)
        )
        .is_empty());
    assert_eq!(
        progress.on_event(
            DownloadEvent::FileChangedHere {
                path: "b\u{7}",
                size: 100
            },
            secs(1)
        ),
        vec!["[00:01]   b\\u{7}: changed in this folder, left as it is"]
    );
    assert_eq!(progress.so_far(), 200);
    assert_eq!(
        progress.on_event(DownloadEvent::SenderPreparing { path: "c\u{1b}" }, secs(1)),
        vec!["[00:01]   c\\u{1b}: the sender is still preparing it; waiting."]
    );
    assert_eq!(
        progress.on_event(
            DownloadEvent::FileCopiedHere {
                path: "c\u{1b}",
                size: 100
            },
            secs(1)
        ),
        vec!["[00:01]   c\\u{1b} (same as a file already downloaded; copied)"]
    );
    assert_eq!(progress.so_far(), 300);
    assert_eq!(
        progress.on_event(DownloadEvent::NoLongerInStick { count: 1 }, secs(2)),
        vec!["[00:02] 1 file no longer in the stick; kept in this folder."]
    );

    let report = DownloadReport {
        seq: 1,
        files: 3,
        bytes: 300,
        received: 100,
        already_here: 1,
        changed_here: 2,
        usage: PathUsage::default(),
    };
    assert_eq!(
        Progress::summary(&report, "/out", secs(3), secs(3))[2],
        "        Not downloaded: 1 file already here; 2 files changed here, left untouched."
    );
}

#[test]
fn a_later_download_says_both_when_and_how_long() {
    let report = DownloadReport {
        seq: 1,
        files: 1,
        bytes: 10,
        received: 10,
        already_here: 0,
        changed_here: 0,
        usage: PathUsage::default(),
    };
    assert!(Progress::summary(&report, "/out", secs(95), secs(2))[0]
        .starts_with("[01:35] Done in 00:02: 1 file in /out"));
}
