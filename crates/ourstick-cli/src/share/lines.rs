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
    Some(match now.readers {
        0 => format!("[{at}] Nobody is downloading now."),
        1 => format!(
            "[{at}] Someone is downloading ({}/s).",
            bytes(now.bytes_per_second)
        ),
        readers => format!(
            "[{at}] {readers} are downloading ({}/s).",
            bytes(now.bytes_per_second)
        ),
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

/// Lines for what the watcher did. Nothing when signs of change changed
/// nothing (e.g. a file saved with the same content).
pub(super) fn watch_lines(
    event: &WatchEvent<'_>,
    elapsed: Duration,
    timing: WatchTiming,
) -> Vec<String> {
    let at = clock(elapsed);
    match event {
        WatchEvent::Unchanged => Vec::new(),
        WatchEvent::Ejected => vec![format!(
            "[{at}] The stick's drive is not there (ejected?); readers keep getting \
             the last version until it is back."
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
            let mut lines = vec![format!(
                "[{at}] Folder changed: {}. Readers now get version {}.",
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
            manifest: ourstick_core::Manifest::empty(),
            manifest_hash: [0; 32],
            seq,
            record: key.sign_record(seq, [0; 32]),
            new_version: true,
            reused: 0,
        }
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
