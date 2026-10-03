//! What a reporter sends, and when.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::*;
use crate::clock::FixedClock;
use crate::report::open;
use crate::stick_id::StickSigningKey;
use crate::store::{JoinedStick, MemoryStore, StickKind};
use crate::ManifestHint;

/// An owner that takes reports, or is away.
#[derive(Default)]
struct Owner {
    sent: Mutex<Vec<Vec<u8>>>,
    away: AtomicBool,
    /// Held by a test: a report waits for it, as one slow to reach the
    /// owner.
    gate: tokio::sync::Mutex<()>,
}

impl ReportTo for Owner {
    async fn report(&self, _ticket: &Ticket, sealed: &[u8]) -> Result<(), TransportError> {
        let _passed = self.gate.lock().await;
        if self.away.load(Ordering::SeqCst) {
            return Err(TransportError::Unreachable("away".to_owned()));
        }
        self.sent.lock().unwrap().push(sealed.to_vec());
        Ok(())
    }
}

impl Owner {
    /// What each report sent says.
    fn told(&self, ticket: &Ticket) -> Vec<ReaderReport> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|sealed| {
                open(sealed, &ticket.stick_id, &ticket.stick_key)
                    .unwrap()
                    .report
            })
            .collect()
    }
}

fn ticket() -> Ticket {
    Ticket {
        stick_id: StickSigningKey::from_bytes(&[4; 32]).stick_id(),
        node_id: crate::ticket::NodeId([5; 32]),
        manifest_hint: ManifestHint {
            seq: 1,
            hash: [0; 32],
        },
        address_hints: Vec::new(),
        stick_key: crate::cipher::StickKey::from_bytes([6; 32]),
    }
}

fn reporter(name: Option<&str>) -> (Reporter, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    store
        .create_device(&crate::store::Device {
            node_id: None,
            secret_backend: crate::store::SecretBackend::SecretService,
            created_at_ms: 1,
        })
        .unwrap();
    store.set_receiver_name(name).unwrap();
    store
        .add_joined_stick(&JoinedStick {
            kind: StickKind::App,
            ..JoinedStick::new(ticket().stick_id, "/d/x", 1)
        })
        .unwrap();
    let reporter = Reporter::new(
        ticket(),
        ReaderKey::generate(),
        store.clone(),
        Arc::new(FixedClock::new(1_000)),
    )
    .every(EVERY);
    (reporter, store)
}

fn download(event: DownloadEvent<'static>) -> ReceiveEvent<'static> {
    ReceiveEvent::Download(event)
}

/// The reporter's 30 seconds, made short.
const EVERY: Duration = Duration::from_millis(400);

/// Long enough for a report to go; well short of [`EVERY`].
const A_MOMENT: Duration = Duration::from_millis(60);

#[tokio::test(flavor = "multi_thread")]
async fn a_round_is_told_as_it_goes_and_when_it_ends() {
    let (reporter, _store) = reporter(Some("Μαρία"));
    let owner = Owner::default();
    let pump = reporter.pump(&owner);
    tokio::pin!(pump);
    let mut step = async |wait: Duration| {
        tokio::select! {
            () = &mut pump => unreachable!(),
            () = tokio::time::sleep(wait) => {}
        }
    };
    // Nothing before a version is known.
    reporter.on_event(&download(DownloadEvent::Planned { bytes: 100 }), 0, 100);
    step(A_MOMENT).await;
    assert!(owner.told(&ticket()).is_empty());
    reporter.on_event(&download(DownloadEvent::NewVersion { seq: 9 }), 0, 100);
    reporter.on_event(&download(DownloadEvent::Planned { bytes: 100 }), 0, 100);
    step(A_MOMENT).await;
    // Progress alone waits 30 s.
    reporter.on_event(
        &download(DownloadEvent::Progress {
            path: "a",
            received: 40,
        }),
        40,
        100,
    );
    step(A_MOMENT).await;
    assert_eq!(owner.told(&ticket()).len(), 1);
    step(EVERY + A_MOMENT).await;
    reporter.on_event(&ReceiveEvent::UpToDate { seq: 9 }, 100, 100);
    step(A_MOMENT).await;
    let told = owner.told(&ticket());
    let states: Vec<ReportState> = told.iter().map(|report| report.state).collect();
    assert_eq!(
        states,
        [
            ReportState::Downloading { percent: 0 },
            ReportState::Downloading { percent: 40 },
            ReportState::UpToDate,
        ]
    );
    assert!(told
        .iter()
        .all(|report| report.seq == 9 && report.everything));
    assert!(told
        .windows(2)
        .all(|pair| pair[0].counter < pair[1].counter));
    assert_eq!(told[0].name.as_deref(), Some("Μαρία"));
    assert_eq!(told[0].kind, ComputerKind::this());
    // A name changed meanwhile is in the next report.
    assert!(told[1..]
        .iter()
        .all(|report| report.name.as_deref() == Some("Μαρία")));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_could_not_go_goes_later_and_the_sender_away_is_not_told() {
    let (reporter, store) = reporter(None);
    store
        .set_left_out(&ticket().stick_id, &["Films".to_owned()])
        .unwrap();
    let owner = Owner::default();
    owner.away.store(true, Ordering::SeqCst);
    let pump = reporter.pump(&owner);
    tokio::pin!(pump);
    let mut step = async |wait: Duration| {
        tokio::select! {
            () = &mut pump => unreachable!(),
            () = tokio::time::sleep(wait) => {}
        }
    };
    reporter.on_event(&ReceiveEvent::UpToDate { seq: 3 }, 0, 0);
    step(A_MOMENT).await;
    assert!(owner.told(&ticket()).is_empty());
    // The sender away: a wait for it is never sent, and nothing is lost.
    reporter.on_event(
        &ReceiveEvent::Waiting {
            why: Waiting::Sender("off".to_owned()),
            after: Duration::from_secs(5),
        },
        0,
        0,
    );
    owner.away.store(false, Ordering::SeqCst);
    step(EVERY + A_MOMENT).await;
    assert!(owner.told(&ticket()).is_empty());
    // Back: up to date again, told at once.
    reporter.on_event(&ReceiveEvent::UpToDate { seq: 3 }, 0, 0);
    step(A_MOMENT).await;
    let told = owner.told(&ticket());
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].state, ReportState::UpToDate);
    // Something left out: the owner learns that, not what.
    assert!(!told[0].everything);
    // A wait the owner can hear of.
    reporter.on_event(
        &ReceiveEvent::Waiting {
            why: Waiting::DiskFull,
            after: Duration::from_secs(5),
        },
        0,
        0,
    );
    step(A_MOMENT).await;
    assert_eq!(
        owner.told(&ticket())[1].state,
        ReportState::Waiting(WaitReason::NoRoom)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_and_forgetting_are_sent_at_once_within_a_deadline() {
    let (reporter, _store) = reporter(None);
    let owner = Owner::default();
    reporter.set_version(2);
    reporter
        .send_now(&owner, ReportState::Forgotten, Duration::from_secs(3))
        .await
        .unwrap();
    let told = owner.told(&ticket());
    assert_eq!((told[0].seq, told[0].state), (2, ReportState::Forgotten));
    owner.away.store(true, Ordering::SeqCst);
    assert!(reporter
        .send_now(&owner, ReportState::Paused, Duration::from_secs(3))
        .await
        .is_err());
}

#[test]
fn waits_are_told_by_a_few_reasons_and_percents_stay_below_100() {
    assert_eq!(wait_reason(&Waiting::Folder), WaitReason::FolderMissing);
    assert_eq!(
        wait_reason(&Waiting::Room {
            needed: 2,
            available: 1
        }),
        WaitReason::NoRoom
    );
    assert_eq!(wait_reason(&Waiting::NoLongerShared), WaitReason::Other);
    let percent = |done, total| match noted(
        &download(DownloadEvent::Progress {
            path: "a",
            received: 0,
        }),
        done,
        total,
    ) {
        Some((_, ReportState::Downloading { percent }, false)) => percent,
        other => panic!("{other:?}"),
    };
    assert_eq!(percent(0, 0), 0);
    assert_eq!(percent(100, 100), 99);
    assert_eq!(percent(200, 100), 99);
    assert_eq!(percent(u64::MAX, u64::MAX), 99);
    assert_eq!(percent(1, 3), 33);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_while_a_report_goes_still_goes_at_once() {
    let (reporter, _store) = reporter(None);
    let owner = Owner::default();
    let pump = reporter.pump(&owner);
    tokio::pin!(pump);
    let mut step = async |wait: Duration| {
        tokio::select! {
            () = &mut pump => unreachable!(),
            () = tokio::time::sleep(wait) => {}
        }
    };
    // A change before its version is known goes once the version is.
    reporter.on_event(&download(DownloadEvent::Planned { bytes: 10 }), 0, 10);
    step(A_MOMENT).await;
    assert!(owner.told(&ticket()).is_empty());
    let held = owner.gate.lock().await;
    reporter.on_event(&download(DownloadEvent::NewVersion { seq: 4 }), 0, 10);
    // It is on its way (held) when the round ends.
    step(A_MOMENT).await;
    reporter.on_event(&ReceiveEvent::UpToDate { seq: 4 }, 10, 10);
    drop(held);
    step(A_MOMENT * 2).await;
    let states: Vec<ReportState> = owner
        .told(&ticket())
        .iter()
        .map(|report| report.state)
        .collect();
    assert_eq!(
        states,
        [
            ReportState::Downloading { percent: 0 },
            ReportState::UpToDate
        ]
    );
}
