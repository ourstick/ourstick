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
    /// The relay limit, while data arrives through the relay.
    relay_limit: Option<u64>,
}

/// A bit rate for humans, e.g. `10 Mbps`.
fn mbps(bits_per_second: u64) -> String {
    let mbps = bits_per_second as f64 / 1_000_000.0;
    if mbps.fract() == 0.0 {
        format!("{mbps:.0} Mbps")
    } else {
        format!("{mbps:.1} Mbps")
    }
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
            DownloadEvent::RelayLimited { bits_per_second } => {
                self.relay_limit = Some(bits_per_second);
                vec![format!(
                    "[{at}] No direct connection found; continuing through the relay, \
                     up to {} on the free relays. A direct connection is still looked for.",
                    mbps(bits_per_second)
                )]
            }
            DownloadEvent::PathChanged { path } => {
                self.path = Some(path);
                if path == PathKind::Direct {
                    self.relay_limit = None;
                }
                vec![format!("[{at}] Now: {}.", describe(path))]
            }
            DownloadEvent::Manifest {
                files,
                bytes: total,
                unchanged,
                partial,
                ..
            } => {
                self.total = total;
                let note = if partial {
                    " so far; the sender is still reading its folder"
                } else if unchanged {
                    " (no changes since last time)"
                } else {
                    ""
                };
                vec![format!("[{at}] {files} files, {}{note}.", bytes(total))]
            }
            DownloadEvent::WaitingForMore { .. } => vec![format!(
                "[{at}] Everything listed so far is here; waiting for the sender to list more."
            )],
            DownloadEvent::OwnerUnreachable { reason } => vec![format!(
                "[{at}] The owner cannot be reached ({}); still trying.",
                untrusted(&reason.to_string())
            )],
            DownloadEvent::OwnerReachable => {
                vec![format!("[{at}] The owner can be reached again.")]
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
                let percent = (now.min(self.total) * 100)
                    .checked_div(self.total)
                    .unwrap_or(100);
                let path = match (self.path, self.relay_limit) {
                    // The limit stays in view while it applies (ADR 0006).
                    (Some(PathKind::Relay), Some(limit)) => {
                        format!(", relay, up to {}", mbps(limit))
                    }
                    (Some(path), _) => format!(", {path}"),
                    (None, _) => String::new(),
                };
                vec![format!(
                    "[{at}] {} of {} ({percent}%), {speed}{path}",
                    bytes(now),
                    bytes(self.total)
                )]
            }
            DownloadEvent::ConnectionLost { reason } => vec![format!(
                "[{at}] The connection was lost ({}); connecting again (the download goes on \
                 where it was).",
                untrusted(reason)
            )],
            DownloadEvent::LookingForDirect => vec![format!(
                "[{at}] Through the relay for a while: connecting again to look for a direct path \
                 (the download goes on where it was)."
            )],
            DownloadEvent::NewVersion { seq } => vec![format!(
                "[{at}] The owner shared a new version ({seq}) meanwhile; going on with it."
            )],
            DownloadEvent::FileAlreadyHere { size, .. } => {
                self.finished += size;
                Vec::new()
            }
            DownloadEvent::FileChangedHere { path, size } => {
                self.finished += size;
                vec![format!(
                    "[{at}]   {}: changed in this folder, left as it is",
                    untrusted(path)
                )]
            }
            // Counted when it was received.
            DownloadEvent::ReceivedButChangedHere { path } => vec![format!(
                "[{at}]   {}: changed in this folder while it downloaded, left as it is",
                untrusted(path)
            )],
            DownloadEvent::FileReceived { size, received, .. } => {
                self.finished += size;
                self.finished_network += received;
                self.current_size = 0;
                self.current_local = 0;
                self.current_received = 0;
                Vec::new()
            }
            DownloadEvent::FileCopiedHere { path, size } => {
                self.finished += size;
                vec![format!(
                    "[{at}]   {} (same as a file already downloaded; copied)",
                    untrusted(path)
                )]
            }
            DownloadEvent::SenderPreparing { path } => vec![format!(
                "[{at}]   {}: the sender is still preparing it; waiting.",
                untrusted(path)
            )],
            DownloadEvent::NoLongerInStick { count: files } => vec![format!(
                "[{at}] {} no longer in the stick; kept in this folder.",
                count(files)
            )],
            // Counted when it was received; it may come after the next
            // file started.
            DownloadEvent::FileDone {
                path,
                already_local,
                received,
            } => {
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
    /// `at` is the time since `get` started, `took` how long this download
    /// took (they differ with `--follow`).
    pub fn summary(
        report: &DownloadReport,
        output: &str,
        at: Duration,
        took: Duration,
    ) -> Vec<String> {
        let mut lines = vec![
            format!(
                "[{}] Done in {}: {} in {output} ({} received, average {}).",
                clock(at),
                clock(took),
                count(report.files),
                bytes(report.received),
                rate(report.received, took)
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
                "{} changed here, left untouched",
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
mod tests;
