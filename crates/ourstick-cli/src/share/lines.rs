//! The lines `share` prints: the network, what changed, who downloads.

use std::time::Duration;

use ourstick_core::transport::{describe_network, NetworkReport, ReaderActivity};
use ourstick_core::watcher::{WatchEvent, WatchTiming};

use crate::display::{bytes, untrusted};
use crate::progress::clock;

/// The "This network" line: what was measured, or that it is not known yet;
/// never nothing.
pub(super) fn network_line(report: Option<&NetworkReport>) -> String {
    match report {
        Some(report) => format!("This network: {}.", describe_network(report)),
        None => "This network: not measured yet (readers still connect; \
                 `get` explains the path it uses)."
            .to_owned(),
    }
}

/// How often `share` says how far reading the folder has come.
const CATALOGUE_EVERY: Duration = Duration::from_secs(10);

/// A line about reading the folder, every [`CATALOGUE_EVERY`] at most, and
/// once it is done; `last` is when the previous one was printed.
pub(super) fn catalogue_line(
    last: &mut Option<Duration>,
    done: usize,
    total: usize,
    elapsed: Duration,
) -> Option<String> {
    let finished = done == total;
    if !finished && last.is_some_and(|last| elapsed < last + CATALOGUE_EVERY) {
        return None;
    }
    *last = Some(elapsed);
    let at = clock(elapsed);
    Some(if finished {
        format!("[{at}] Read all {total} files.")
    } else {
        format!("[{at}] Read {done} of {total} files.")
    })
}

/// A line when the number of readers changes: who downloads now, and how
/// fast. Nothing while it stays the same.
pub(super) fn readers_line(
    before: usize,
    now: &ReaderActivity,
    elapsed: Duration,
) -> Option<String> {
    if now.readers == before {
        return None;
    }
    let at = clock(elapsed);
    // Nothing leaves while their files are got ready: say that, not "0 B/s".
    let how = if now.preparing {
        "getting their files ready".to_owned()
    } else {
        format!("{}/s", bytes(now.bytes_per_second))
    };
    Some(match now.readers {
        0 => format!("[{at}] Nobody is downloading now."),
        1 => format!("[{at}] Someone is downloading ({how})."),
        readers => format!("[{at}] {readers} are downloading ({how})."),
    })
}

/// A line once readers turn out to reach this computer only through the
/// relay: most likely a firewall drops what others send it.
pub(super) fn relay_only_line(
    before: bool,
    now: &ReaderActivity,
    elapsed: Duration,
) -> Option<String> {
    if !now.only_relayed || before {
        return None;
    }
    let fix = if cfg!(windows) {
        "allow OurStick in Windows Security > Firewall & network protection > \
         Allow an app through firewall"
    } else {
        "allow incoming UDP for ourstick in this computer's firewall"
    };
    Some(format!(
        "[{}] Readers reach this computer only through the relay, which is slower \
         and limited. A firewall probably drops what they send; {fix}.",
        clock(elapsed)
    ))
}

/// Said when OurStick begins sending slowly to give way to another program
/// using the disk, and when it stops (ADR 0017, amendment 1); nothing
/// while it stays the same.
pub(super) fn sending_slowly_line(
    before: bool,
    now: &ReaderActivity,
    elapsed: Duration,
) -> Option<String> {
    if now.giving_way == before {
        return None;
    }
    let at = clock(elapsed);
    Some(if now.giving_way {
        format!("[{at}] Sending slowly, so another program can use the disk.")
    } else {
        format!("[{at}] Sending at full speed again.")
    })
}

/// Said once when OurStick begins giving way to another program using the
/// disk, and once when it stops (ADR 0013).
pub(super) fn giving_way_line(now: bool, elapsed: Duration) -> String {
    let at = clock(elapsed);
    if now {
        format!("[{at}] Reading your folder slowly, so another program can use the disk.")
    } else {
        format!("[{at}] Reading at full speed again.")
    }
}

/// Lines for what the watcher did. Nothing when signs of change changed
/// nothing (e.g. a file saved with the same content).
pub(super) fn watch_lines(
    event: &WatchEvent<'_>,
    elapsed: Duration,
    timing: WatchTiming,
) -> Vec<String> {
    let at = clock(elapsed);
    match event {
        // Told by `catalogue_line`, now and then; `share <folder>` links
        // nothing (linked entries are the app's).
        WatchEvent::Unchanged | WatchEvent::Reading { .. } | WatchEvent::Roots { .. } => Vec::new(),
        WatchEvent::Ejected => vec![format!(
            "[{at}] The stick's drive is not there (ejected?); readers keep getting \
             the last version until it is back."
        )],
        WatchEvent::GivingWay(now) => vec![giving_way_line(*now, elapsed)],
        WatchEvent::SeenSlowly => vec![format!(
            "[{at}] Some folders cannot be watched (too many?); their changes are \
             seen every {} s.",
            timing.retry.as_secs()
        )],
        WatchEvent::StillChanging => {
            vec![format!("[{at}] A file is still being written; waiting…")]
        }
        WatchEvent::Failed { error } => vec![format!(
            "[{at}] Could not share the change: {}. Trying again at the next change, \
             or in {} s.",
            untrusted(&error.to_string()),
            timing.retry.as_secs()
        )],
        WatchEvent::Published {
            changes,
            publication,
        } => {
            let mut parts = Vec::new();
            for (count, what) in [
                (changes.added.len(), "added"),
                (changes.changed.len(), "changed"),
                (changes.removed.len(), "removed"),
            ] {
                if count > 0 {
                    parts.push(format!("{count} {what}"));
                }
            }
            let still = if publication.manifest.partial {
                " (still reading the folder)"
            } else {
                ""
            };
            let mut lines = vec![format!(
                "[{at}] Folder changed: {}. Readers now get version {}{still}.",
                parts.join(", "),
                publication.seq
            )];
            let named = changes
                .added
                .iter()
                .map(|path| ("+", path))
                .chain(changes.changed.iter().map(|path| ("~", path)))
                .chain(changes.removed.iter().map(|path| ("-", path)));
            let total = changes.added.len() + changes.changed.len() + changes.removed.len();
            for (mark, path) in named.take(MAX_NAMED) {
                lines.push(format!("        {mark} {}", untrusted(path)));
            }
            if total > MAX_NAMED {
                lines.push(format!("        … and {} more", total - MAX_NAMED));
            }
            lines
        }
    }
}

/// At most this many changed paths are listed after a change.
const MAX_NAMED: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    fn publication(seq: u64) -> ourstick_core::share::Publication {
        let key = ourstick_core::StickSigningKey::from_bytes(&[1; 32]);
        ourstick_core::share::Publication {
            manifest: std::sync::Arc::new(ourstick_core::Manifest::empty()),
            manifest_hash: [0; 32],
            seq,
            record: key.sign_record(seq, [0; 32]),
            new_version: true,
            reused: 0,
        }
    }

    #[test]
    fn giving_way_is_said_when_it_starts_and_when_it_stops() {
        let at = Duration::from_secs(75);
        assert_eq!(
            watch_lines(&WatchEvent::GivingWay(true), at, WatchTiming::default()),
            ["[01:15] Reading your folder slowly, so another program can use the disk."]
        );
        assert_eq!(
            giving_way_line(false, at),
            "[01:15] Reading at full speed again."
        );
    }

    #[test]
    pub(super) fn watch_lines_say_what_changed_briefly_and_safely() {
        let timing = WatchTiming::default();
        let at = Duration::from_secs(75);
        let changes = ourstick_core::watcher::ChangeSummary {
            added: (1..=6).map(|index| format!("new{index}.txt")).collect(),
            changed: vec!["\u{1b}[31mred.txt".to_owned()],
            removed: vec![],
        };
        let publication = publication(42);
        let lines = watch_lines(
            &WatchEvent::Published {
                changes: &changes,
                publication: &publication,
            },
            at,
            timing,
        );
        assert_eq!(
            lines,
            vec![
                "[01:15] Folder changed: 6 added, 1 changed. Readers now get version 42.",
                "        + new1.txt",
                "        + new2.txt",
                "        + new3.txt",
                "        + new4.txt",
                "        + new5.txt",
                "        … and 2 more",
            ]
        );
        assert!(watch_lines(&WatchEvent::Unchanged, at, timing).is_empty());
        let failed = watch_lines(
            &WatchEvent::Failed {
                error: &ourstick_core::share::ShareError::NotOwned,
            },
            at,
            timing,
        );
        assert_eq!(
            failed,
            vec![
                "[01:15] Could not share the change: this device does not own that stick. \
                 Trying again at the next change, or in 30 s."
            ]
        );
    }

    #[test]
    fn changed_names_are_escaped() {
        let changes = ourstick_core::watcher::ChangeSummary {
            added: vec![],
            changed: vec!["\u{1b}[31mred.txt".to_owned()],
            removed: vec![],
        };
        let publication = publication(1);
        let lines = watch_lines(
            &WatchEvent::Published {
                changes: &changes,
                publication: &publication,
            },
            Duration::ZERO,
            WatchTiming::default(),
        );
        assert_eq!(lines[1], "        ~ \\u{1b}[31mred.txt");
    }

    #[test]
    fn readers_are_announced_when_their_number_changes() {
        let at = Duration::from_secs(5);
        let busy = |readers| ReaderActivity {
            readers,
            bytes_per_second: 2_100_000,
            only_relayed: false,
            preparing: false,
            giving_way: false,
        };
        assert_eq!(
            readers_line(0, &busy(1), at).unwrap(),
            "[00:05] Someone is downloading (2.1 MB/s)."
        );
        assert_eq!(readers_line(1, &busy(1), at), None);
        assert_eq!(
            readers_line(1, &busy(2), at).unwrap(),
            "[00:05] 2 are downloading (2.1 MB/s)."
        );
        assert_eq!(
            readers_line(2, &busy(0), at).unwrap(),
            "[00:05] Nobody is downloading now."
        );
        let waiting = ReaderActivity {
            readers: 1,
            preparing: true,
            ..ReaderActivity::default()
        };
        assert_eq!(
            readers_line(0, &waiting, at).unwrap(),
            "[00:05] Someone is downloading (getting their files ready)."
        );
    }

    #[test]
    fn sending_slowly_is_said_when_it_changes() {
        let at = Duration::from_secs(65);
        let slow = ReaderActivity {
            readers: 1,
            giving_way: true,
            ..ReaderActivity::default()
        };
        assert_eq!(
            sending_slowly_line(false, &slow, at).unwrap(),
            "[01:05] Sending slowly, so another program can use the disk."
        );
        assert_eq!(sending_slowly_line(true, &slow, at), None);
        assert_eq!(
            sending_slowly_line(true, &ReaderActivity::default(), at).unwrap(),
            "[01:05] Sending at full speed again."
        );
    }

    #[test]
    fn relay_only_is_said_once() {
        let at = Duration::from_secs(65);
        let relayed = ReaderActivity {
            only_relayed: true,
            ..ReaderActivity::default()
        };
        let line = relay_only_line(false, &relayed, at).unwrap();
        assert!(line.starts_with("[01:05] Readers reach this computer only through the relay"));
        assert!(line.contains("firewall"));
        assert_eq!(relay_only_line(true, &relayed, at), None);
        assert_eq!(relay_only_line(false, &ReaderActivity::default(), at), None);
    }

    #[test]
    fn reading_the_folder_is_told_now_and_then() {
        let mut last = None;
        let at = Duration::from_secs;
        assert_eq!(
            catalogue_line(&mut last, 256, 1000, at(1)).as_deref(),
            Some("[00:01] Read 256 of 1000 files.")
        );
        assert_eq!(catalogue_line(&mut last, 512, 1000, at(5)), None);
        assert_eq!(
            catalogue_line(&mut last, 768, 1000, at(11)).as_deref(),
            Some("[00:11] Read 768 of 1000 files.")
        );
        // The end is always told.
        assert_eq!(
            catalogue_line(&mut last, 1000, 1000, at(12)).as_deref(),
            Some("[00:12] Read all 1000 files.")
        );
    }

    #[test]
    fn network_line_is_never_empty() {
        assert!(network_line(None).starts_with("This network: not measured yet"));
        let report = NetworkReport {
            udp_v4: true,
            udp_v6: false,
            ipv4_mapping_varies: Some(false),
            public_v4: Some("203.0.113.32:35611".parse().unwrap()),
            public_v6: None,
        };
        assert_eq!(
            network_line(Some(&report)),
            format!("This network: {}.", describe_network(&report))
        );
    }
}
