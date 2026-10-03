//! The owner's side of readers' reports (ADR 0015): each opened with the
//! stick's key and kept, and what each line says to the sender.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::blocking::run_blocking;
use crate::cipher::StickKey;
use crate::clock::Clock;
use crate::report::{open, ReportState, WaitReason};
use crate::stick_id::StickId;
use crate::store::{KeepOutcome, KeptReport, Store};

/// Opens each sealed report that comes for `stick_id` and keeps it; calls
/// `on_kept` after each one that changed the list. Ends when the
/// publisher's channel closes. A report that does not open is dropped,
/// logged without its bytes.
pub async fn keep_reports(
    mut reports: mpsc::Receiver<Vec<u8>>,
    store: Arc<dyn Store>,
    stick_id: StickId,
    stick_key: StickKey,
    clock: Arc<dyn Clock>,
    mut on_kept: impl FnMut() + Send,
) {
    while let Some(sealed) = reports.recv().await {
        let opened = match open(&sealed, &stick_id, &stick_key) {
            Ok(opened) => opened,
            Err(error) => {
                tracing::debug!(%error, "a reader's report was not kept");
                continue;
            }
        };
        let store = store.clone();
        let now_ms = clock.now_ms();
        let kept = run_blocking(move || {
            store.keep_report(&stick_id, &opened.reader, &opened.report, now_ms)
        })
        .await;
        match kept {
            Ok(KeepOutcome::Kept) => on_kept(),
            Ok(_) => {}
            Err(error) => tracing::debug!(%error, "a reader's report was not kept"),
        }
    }
}

/// What a line tells the sender of one computer (rule 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Has {
    /// Up to date with the latest complete version, nothing left out.
    All,
    /// Up to date with it, something left out.
    WhatItChose,
    /// Up to date with a version the owner is still reading its folder for
    /// (ADR 0007); `everything` as the report says.
    SoFar {
        everything: bool,
    },
    /// Was up to date with an earlier version: not what changed since.
    Older {
        everything: bool,
    },
    Downloading {
        percent: u8,
    },
    Waiting(WaitReason),
    Paused,
}

impl Has {
    /// Up to date with the latest, all of it or all it chose.
    pub fn is_up_to_date(self) -> bool {
        matches!(self, Self::All | Self::WhatItChose)
    }
}

/// What `kept` tells the sender, given the version the owner serves now
/// (`latest`: its seq and whether it is partial). The one place that
/// decides; `None` for a line that says nothing (forgotten).
pub fn has(kept: &KeptReport, latest: Option<(u64, bool)>) -> Option<Has> {
    Some(match kept.state {
        ReportState::Downloading { percent } => Has::Downloading { percent },
        ReportState::Waiting(reason) => Has::Waiting(reason),
        ReportState::Paused => Has::Paused,
        ReportState::Forgotten => return None,
        ReportState::UpToDate => {
            let everything = kept.everything;
            match latest {
                Some((seq, partial)) if kept.seq == seq => match (partial, everything) {
                    (true, _) => Has::SoFar { everything },
                    (false, true) => Has::All,
                    (false, false) => Has::WhatItChose,
                },
                _ => Has::Older { everything },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{ComputerKind, ReaderId};

    fn kept(seq: u64, state: ReportState, everything: bool) -> KeptReport {
        KeptReport {
            reader: ReaderId::from_bytes([1; 32]),
            counter: 1,
            seq,
            state,
            everything,
            kind: ComputerKind::MacOs,
            name: None,
            first_ms: 1,
            received_ms: 2,
        }
    }

    #[test]
    fn a_line_says_all_of_it_only_for_the_latest_complete_version() {
        let up = |seq, everything| kept(seq, ReportState::UpToDate, everything);
        assert_eq!(has(&up(5, true), Some((5, false))), Some(Has::All));
        assert_eq!(has(&up(5, false), Some((5, false))), Some(Has::WhatItChose));
        // The owner still reads its folder: everything so far.
        assert_eq!(
            has(&up(5, true), Some((5, true))),
            Some(Has::SoFar { everything: true })
        );
        // An earlier version, or one the owner does not know (none served).
        assert_eq!(
            has(&up(4, true), Some((5, false))),
            Some(Has::Older { everything: true })
        );
        assert_eq!(
            has(&up(5, true), None),
            Some(Has::Older { everything: true })
        );
        assert!(Has::All.is_up_to_date() && Has::WhatItChose.is_up_to_date());
        assert!(!Has::SoFar { everything: true }.is_up_to_date());
    }

    #[test]
    fn other_states_say_what_the_reader_does() {
        let latest = Some((5, false));
        assert_eq!(
            has(
                &kept(5, ReportState::Downloading { percent: 60 }, true),
                latest
            ),
            Some(Has::Downloading { percent: 60 })
        );
        assert_eq!(
            has(
                &kept(5, ReportState::Waiting(WaitReason::NoRoom), true),
                latest
            ),
            Some(Has::Waiting(WaitReason::NoRoom))
        );
        assert_eq!(
            has(&kept(5, ReportState::Paused, true), latest),
            Some(Has::Paused)
        );
        assert_eq!(has(&kept(5, ReportState::Forgotten, true), latest), None);
    }

    #[tokio::test]
    async fn reports_that_open_are_kept_and_the_rest_dropped() {
        use crate::report::{seal, ReaderKey, ReaderReport};
        use crate::stick_id::StickSigningKey;
        use crate::store::{MemoryStore, OwnedStick};

        let signing = StickSigningKey::from_bytes(&[2; 32]);
        let stick_id = signing.stick_id();
        let key = StickKey::from_bytes([3; 32]);
        let store = Arc::new(MemoryStore::new());
        store
            .add_owned_stick(&OwnedStick::new(
                stick_id,
                "Holiday".to_owned(),
                "/s",
                crate::cipher::CipherId::XChaCha20Poly1305,
                1,
            ))
            .unwrap();
        let report = ReaderReport {
            counter: 1,
            seq: 5,
            state: ReportState::UpToDate,
            everything: true,
            kind: ComputerKind::Windows,
            name: None,
        };
        let reader = ReaderKey::generate();
        let (sender, receiver) = mpsc::channel(4);
        sender
            .send(seal(&report, &reader, &stick_id, &key).unwrap())
            .await
            .unwrap();
        // Sealed with another key: dropped.
        sender
            .send(seal(&report, &reader, &stick_id, &StickKey::from_bytes([9; 32])).unwrap())
            .await
            .unwrap();
        // The same again: not kept again.
        sender
            .send(seal(&report, &reader, &stick_id, &key).unwrap())
            .await
            .unwrap();
        drop(sender);
        let mut kept = 0;
        keep_reports(
            receiver,
            store.clone(),
            stick_id,
            key,
            Arc::new(crate::clock::FixedClock::new(1_000)),
            || kept += 1,
        )
        .await;
        assert_eq!(kept, 1);
        let lines = store.reports(&stick_id, 1_000).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].reader, reader.reader_id());
        assert_eq!(has(&lines[0], Some((5, false))), Some(Has::All));
    }
}
