//! What `get` prints while it downloads: every line starts with the time since
//! the start, a progress line with speed and path appears every few seconds,
//! and the end says how long it took and how much came direct or via a relay.
//!
//! Pure: the caller passes the elapsed time, so this is tested without a clock.

use std::time::Duration;

use ourstick_core::download::{DownloadEvent, DownloadReport};
use ourstick_core::transport::PathKind;

use crate::display::{bytes, untrusted};

/// How often a progress line is printed during a long file.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(3);

/// `mm:ss`, or `h:mm:ss` after an hour.
pub fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Bytes per second over `elapsed`, for humans.
pub fn rate(count: u64, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return "–".to_owned();
    }
    format!("{}/s", bytes((count as f64 / seconds) as u64))
}

fn describe(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "direct connection",
        PathKind::Relay => "through a relay (slower; a direct path may still be found)",
        PathKind::Unknown => "connection type not known yet",
    }
}

#[derive(Debug, Default)]
pub struct Progress {
    path: Option<PathKind>,
    total: u64,
    /// Sizes of the files finished in this run.
    finished: u64,
    /// Bytes received over the network for the files finished in this run.
    finished_network: u64,
    current_size: u64,
    /// What the current file already had locally, from an interrupted run.
    current_local: u64,
    current_received: u64,
    last_line_at: Duration,
    network_at_last_line: u64,
}

/// "1 file", "2 files".
fn count(files: usize) -> String {
    match files {
        1 => "1 file".to_owned(),
        files => format!("{files} files"),
    }
}

impl Progress {
    /// How much of the stick is here, including what an earlier run left.
    fn so_far(&self) -> u64 {
        self.finished + (self.current_local + self.current_received).min(self.current_size)
    }

    /// The path the data currently arrives on, as last reported.
    pub fn path(&self) -> Option<PathKind> {
        self.path
    }

    /// What came over the network in this run; the speed is based on this.
    fn network(&self) -> u64 {
        self.finished_network + self.current_received
    }

    /// Lines to print for `event`, which happened `elapsed` after the start.
    pub fn on_event(&mut self, event: DownloadEvent<'_>, elapsed: Duration) -> Vec<String> {
        let at = clock(elapsed);
        match event {
            DownloadEvent::Connected { path } => {
                self.path = Some(path);
                vec![format!("[{at}] Connected: {}.", describe(path))]
            }
            DownloadEvent::WaitingForDirect {
                seconds,
                budget_used: false,
            } => vec![format!(
                "[{at}] No direct connection yet: waiting up to {seconds} s for one…"
            )],
            DownloadEvent::WaitingForDirect {
                seconds,
                budget_used: true,
            } => vec![format!(
                "[{at}] Waiting up to {seconds} s for a direct connection…"
            )],
            DownloadEvent::RelayBudgetUsed { bytes: used } => vec![format!(
                "[{at}] {} came through the relay, the most allowed without asking; \
                 reconnecting to look for a direct connection…",
                bytes(used)
            )],
            DownloadEvent::DirectLost => vec![format!(
                "[{at}] The direct connection was lost; reconnecting to find it again…"
            )],
            DownloadEvent::PathChanged { path } => {
                self.path = Some(path);
                vec![format!("[{at}] Now: {}.", describe(path))]
            }
            DownloadEvent::Manifest {
                files,
                bytes: total,
                unchanged,
                ..
            } => {
                self.total = total;
                let note = if unchanged {
                    " (no changes since last time)"
                } else {
                    ""
                };
                vec![format!("[{at}] {files} files, {}{note}.", bytes(total))]
            }
            DownloadEvent::FileStarted {
                size,
                already_local,
                ..
            } => {
                self.current_size = size;
                self.current_local = already_local;
                self.current_received = 0;
                Vec::new()
            }
            DownloadEvent::Progress { received, .. } => {
                self.current_received = received;
                let since = elapsed.saturating_sub(self.last_line_at);
                if since < PROGRESS_EVERY {
                    return Vec::new();
                }
                let now = self.so_far();
                let network = self.network();
                let speed = rate(network.saturating_sub(self.network_at_last_line), since);
                self.last_line_at = elapsed;
                self.network_at_last_line = network;
                let percent = if self.total > 0 {
                    now.min(self.total) * 100 / self.total
                } else {
                    100
                };
                let path = self
                    .path
                    .map(|path| format!(", {path}"))
                    .unwrap_or_default();
                vec![format!(
                    "[{at}] {} of {} ({percent}%), {speed}{path}",
                    bytes(now),
                    bytes(self.total)
                )]
            }
            DownloadEvent::NewVersion { seq } => vec![format!(
                "[{at}] The owner shared a new version ({seq}) meanwhile; going on with it."
            )],
            DownloadEvent::FileAlreadyHere { size, .. } => {
                self.finished += size;
                Vec::new()
            }
            DownloadEvent::FileChangedHere { path, size } => {
                // Also when it changed while downloading: what came counts.
                self.finished += size;
                self.finished_network += self.current_received;
                self.current_size = 0;
                self.current_local = 0;
                self.current_received = 0;
                vec![format!(
                    "[{at}]   {}: changed in this folder, left as it is",
                    untrusted(path)
                )]
            }
            DownloadEvent::NoLongerInStick { count: files } => vec![format!(
                "[{at}] {} no longer in the stick; kept in this folder.",
                count(files)
            )],
            DownloadEvent::FileDone {
                path,
                already_local,
                received,
            } => {
                self.finished += self.current_size;
                self.finished_network += received;
                self.current_local = 0;
                self.current_received = 0;
                let resumed = if already_local > 0 {
                    format!(", resumed after {}", bytes(already_local))
                } else {
                    String::new()
                };
                vec![format!(
                    "[{at}]   {} ({} received{resumed})",
                    untrusted(path),
                    bytes(received)
                )]
            }
        }
    }

    /// The closing lines of a finished download.
    pub fn summary(report: &DownloadReport, output: &str, elapsed: Duration) -> Vec<String> {
        let at = clock(elapsed);
        let mut lines = vec![
            format!(
                "[{at}] Done in {at}: {} files in {output} ({} received, average {}).",
                report.files,
                bytes(report.received),
                rate(report.received, elapsed)
            ),
            format!(
                "        Received directly: {} ({} over IPv6), through a relay: {}.",
                bytes(report.usage.direct),
                bytes(report.usage.direct_ipv6),
                bytes(report.usage.relay)
            ),
        ];
        let mut kept = Vec::new();
        if report.already_here > 0 {
            kept.push(format!("{} already here", count(report.already_here)));
        }
        if report.changed_here > 0 {
            kept.push(format!(
                "{} changed here and left as they were",
                count(report.changed_here)
            ));
        }
        if !kept.is_empty() {
            lines.push(format!("        Not downloaded: {}.", kept.join("; ")));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
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
    fn relay_policy_events_say_what_happens() {
        let mut progress = Progress::default();
        let mut line = |event| progress.on_event(event, secs(5)).join("\n");
        assert_eq!(
            line(DownloadEvent::WaitingForDirect {
                seconds: 5,
                budget_used: false
            }),
            "[00:05] No direct connection yet: waiting up to 5 s for one…"
        );
        assert_eq!(
            line(DownloadEvent::RelayBudgetUsed { bytes: 104_000_000 }),
            "[00:05] 104.0 MB came through the relay, the most allowed without asking; \
             reconnecting to look for a direct connection…"
        );
        assert_eq!(
            line(DownloadEvent::WaitingForDirect {
                seconds: 15,
                budget_used: true
            }),
            "[00:05] Waiting up to 15 s for a direct connection…"
        );
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
        let lines = Progress::summary(&report, "/Users/a/received", secs(252));
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
            Progress::summary(&report, "/out", secs(3))[2],
            "        Not downloaded: 1 file already here; 2 files changed here and left as they were."
        );
    }
}
