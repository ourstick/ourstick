//! The progress line with small files fetched together (ADR 0016).

use super::*;
use std::time::Duration;

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn small_files_fetched_together_move_the_line_too() {
    // ADR 0016: they tell only `FileReceived`.
    let mut progress = Progress::default();
    progress.on_event(
        DownloadEvent::Manifest {
            seq: 1,
            files: 1000,
            bytes: 4_000_000,
            unchanged: false,
            partial: false,
        },
        secs(0),
    );
    let mut lines = Vec::new();
    for index in 0..1000u64 {
        lines.extend(progress.on_event(
            DownloadEvent::FileReceived {
                path: "a.txt",
                size: 4_000,
                already_local: 0,
                received: 4_000,
            },
            Duration::from_millis(index * 10),
        ));
    }
    // About one line every 3 seconds over 10 seconds, and the last says
    // how far.
    assert!((2..=4).contains(&lines.len()), "{lines:?}");
    assert!(lines.iter().all(|line| line.contains(" of 4.0 MB (")));
}
