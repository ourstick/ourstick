//! Readers' reports on owned sticks, the receiver's name, report counters
//! (ADR 0015): the same in every [`Store`].

use super::*;
use crate::report::{ComputerKind, ReaderId, ReaderReport, ReportState, WaitReason};
use crate::store::reports::{KEPT_MS, MAX_READERS, SILENT_MS};
use crate::store::{KeepOutcome, StickKind};

const NOW: u64 = 1_800_000_000_000;

fn reader(seed: u16) -> ReaderId {
    let mut bytes = [0u8; 32];
    bytes[..2].copy_from_slice(&seed.to_be_bytes());
    ReaderId::from_bytes(bytes)
}

fn report(counter: u64, state: ReportState) -> ReaderReport {
    ReaderReport {
        counter,
        seq: 7,
        state,
        everything: true,
        kind: ComputerKind::MacOs,
        name: None,
    }
}

fn shared(store: &dyn Store, seed: u8) -> StickId {
    store.add_owned_stick(&owned(seed, 10)).unwrap();
    owner(seed).stick_id()
}

fn a_readers_latest_report_is_kept_and_an_old_one_changes_nothing(store: &dyn Store) {
    let id = shared(store, 1);
    let keep =
        |counter, state, now| store.keep_report(&id, &reader(1), &report(counter, state), now);
    let percent = |percent| ReportState::Downloading { percent };
    assert_eq!(keep(10, percent(5), NOW).unwrap(), KeepOutcome::Kept);
    assert_eq!(keep(20, percent(60), NOW + 1).unwrap(), KeepOutcome::Kept);
    // Sent again, or overtaken: nothing changes.
    assert_eq!(
        keep(10, ReportState::UpToDate, NOW + 2).unwrap(),
        KeepOutcome::Old
    );
    assert_eq!(
        keep(20, ReportState::UpToDate, NOW + 2).unwrap(),
        KeepOutcome::Old
    );
    let lines = store.reports(&id, NOW + 3).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].state, percent(60));
    assert_eq!((lines[0].first_ms, lines[0].received_ms), (NOW, NOW + 1));
    // Every field travels.
    let told = ReaderReport {
        counter: 30,
        seq: 9,
        state: ReportState::Waiting(WaitReason::NoRoom),
        everything: false,
        kind: ComputerKind::Windows,
        name: Some("Μαρία".to_owned()),
    };
    store.keep_report(&id, &reader(1), &told, NOW + 4).unwrap();
    let line = &store.reports(&id, NOW + 4).unwrap()[0];
    assert_eq!(
        (
            line.counter,
            line.seq,
            line.state,
            line.everything,
            line.kind,
            line.name.as_deref()
        ),
        (
            30,
            9,
            told.state,
            false,
            ComputerKind::Windows,
            Some("Μαρία")
        )
    );
    assert_eq!(line.first_ms, NOW);
    // Latest first.
    store
        .keep_report(&id, &reader(2), &report(1, ReportState::Paused), NOW + 5)
        .unwrap();
    let readers: Vec<ReaderId> = store
        .reports(&id, NOW + 5)
        .unwrap()
        .iter()
        .map(|line| line.reader)
        .collect();
    assert_eq!(readers, [reader(2), reader(1)]);
}

fn a_removed_line_comes_back_only_with_a_newer_report(store: &dyn Store) {
    let id = shared(store, 1);
    let keep =
        |seed, counter, state| store.keep_report(&id, &reader(seed), &report(counter, state), NOW);
    keep(1, 10, ReportState::UpToDate).unwrap();
    keep(2, 10, ReportState::UpToDate).unwrap();
    store.hide_report(&id, &reader(1)).unwrap();
    assert_eq!(store.reports(&id, NOW).unwrap().len(), 1);
    assert_eq!(
        keep(1, 10, ReportState::UpToDate).unwrap(),
        KeepOutcome::Old
    );
    assert_eq!(store.reports(&id, NOW).unwrap().len(), 1);
    assert_eq!(
        keep(1, 11, ReportState::UpToDate).unwrap(),
        KeepOutcome::Kept
    );
    assert_eq!(store.reports(&id, NOW).unwrap().len(), 2);
    // "Clear this list".
    store.hide_reports(&id).unwrap();
    assert!(store.reports(&id, NOW).unwrap().is_empty());
    // A reader that forgot the stick: its line goes, its counter stays.
    keep(2, 20, ReportState::UpToDate).unwrap();
    assert_eq!(
        keep(2, 21, ReportState::Forgotten).unwrap(),
        KeepOutcome::Kept
    );
    assert!(store.reports(&id, NOW).unwrap().is_empty());
    assert_eq!(
        keep(2, 20, ReportState::UpToDate).unwrap(),
        KeepOutcome::Old
    );
    // Forgotten from a reader never heard: nothing to take off.
    assert_eq!(
        keep(3, 1, ReportState::Forgotten).unwrap(),
        KeepOutcome::Ignored
    );
}

fn a_flood_fills_only_the_free_places(store: &dyn Store) {
    let id = shared(store, 1);
    let keep = |seed: u16, now| {
        store.keep_report(&id, &reader(seed), &report(1, ReportState::UpToDate), now)
    };
    for seed in 0..MAX_READERS as u16 {
        assert_eq!(
            keep(seed, NOW + u64::from(seed)).unwrap(),
            KeepOutcome::Kept
        );
    }
    // Full, everyone speaking: a new reader is refused, nobody pushed off.
    assert_eq!(keep(9_000, NOW + 1_000).unwrap(), KeepOutcome::Full);
    assert_eq!(store.reports(&id, NOW + 1_000).unwrap().len(), MAX_READERS);
    // With no one silent for 30 days, a hidden line makes room.
    store.hide_report(&id, &reader(150)).unwrap();
    assert_eq!(keep(9_000, NOW + 1_000).unwrap(), KeepOutcome::Kept);
    assert_eq!(keep(9_001, NOW + 1_000).unwrap(), KeepOutcome::Full);
    // A reader silent for 30 days makes room before a hidden one: the
    // longest silent goes, the hidden line keeps its counter.
    store.hide_report(&id, &reader(151)).unwrap();
    let later = NOW + SILENT_MS;
    assert_eq!(keep(9_001, later).unwrap(), KeepOutcome::Kept);
    let readers: Vec<ReaderId> = store
        .reports(&id, later)
        .unwrap()
        .iter()
        .map(|line| line.reader)
        .collect();
    assert!(!readers.contains(&reader(0)) && readers.contains(&reader(1)));
    assert_eq!(keep(151, later).unwrap(), KeepOutcome::Old);
}

fn the_owners_clock_set_back_changes_nothing_kept(store: &dyn Store) {
    let id = shared(store, 1);
    store
        .keep_report(&id, &reader(1), &report(1, ReportState::UpToDate), NOW)
        .unwrap();
    // A newer report, the clock an hour behind: kept, first heard not
    // after it.
    let behind = NOW - 3_600_000;
    assert_eq!(
        store
            .keep_report(&id, &reader(1), &report(2, ReportState::Paused), behind)
            .unwrap(),
        KeepOutcome::Kept
    );
    let line = &store.reports(&id, behind).unwrap()[0];
    assert_eq!((line.first_ms, line.received_ms), (behind, behind));
    // Forgotten with a counter not above the last: an old report, nothing
    // taken off.
    assert_eq!(
        store
            .keep_report(&id, &reader(1), &report(2, ReportState::Forgotten), behind)
            .unwrap(),
        KeepOutcome::Old
    );
    assert_eq!(store.reports(&id, behind).unwrap().len(), 1);
    // Hiding on a stick not shared here does nothing, and fails nothing.
    let other = owner(9).stick_id();
    store.hide_report(&other, &reader(1)).unwrap();
    store.hide_reports(&other).unwrap();
    assert!(store.reports(&other, NOW).unwrap().is_empty());
}

fn lines_older_than_ninety_days_are_not_shown_and_go(store: &dyn Store) {
    let id = shared(store, 1);
    store
        .keep_report(&id, &reader(1), &report(1, ReportState::UpToDate), NOW)
        .unwrap();
    assert_eq!(store.reports(&id, NOW + KEPT_MS).unwrap().len(), 1);
    assert!(store.reports(&id, NOW + KEPT_MS + 1).unwrap().is_empty());
    // Gone at the next report: its old counter no longer holds it back.
    store
        .keep_report(
            &id,
            &reader(2),
            &report(1, ReportState::UpToDate),
            NOW + KEPT_MS + 1,
        )
        .unwrap();
    assert_eq!(
        store
            .keep_report(
                &id,
                &reader(1),
                &report(1, ReportState::UpToDate),
                NOW + KEPT_MS + 1
            )
            .unwrap(),
        KeepOutcome::Kept
    );
}

fn reports_are_only_for_sticks_shared_here_and_go_with_them(store: &dyn Store) {
    let other = owner(5).stick_id();
    assert!(matches!(
        store.keep_report(&other, &reader(1), &report(1, ReportState::UpToDate), NOW),
        Err(StoreError::StickNotFound)
    ));
    let id = shared(store, 1);
    store
        .keep_report(&id, &reader(1), &report(1, ReportState::UpToDate), NOW)
        .unwrap();
    // A new link: the old stick's lines go with it.
    store.replace_owned_stick(&id, &owned(2, 20), NOW).unwrap();
    assert!(store.reports(&id, NOW).unwrap().is_empty());
    let new = owner(2).stick_id();
    store
        .keep_report(&new, &reader(1), &report(1, ReportState::UpToDate), NOW)
        .unwrap();
    store.mark_removing(&new).unwrap();
    assert!(matches!(
        store.keep_report(&new, &reader(2), &report(1, ReportState::UpToDate), NOW),
        Err(StoreError::StickNotFound)
    ));
    store.remove_owned_stick(&new).unwrap();
    assert!(store.reports(&new, NOW).unwrap().is_empty());
    // What breaks the rules is refused at the door.
    let id = shared(store, 3);
    let bad = [
        ReaderReport {
            name: Some("a\u{202e}b".to_owned()),
            ..report(1, ReportState::UpToDate)
        },
        report(1, ReportState::Downloading { percent: 100 }),
        report(u64::MAX, ReportState::UpToDate),
    ];
    for told in bad {
        assert!(store.keep_report(&id, &reader(1), &told, NOW).is_err());
    }
}

fn the_receivers_name_is_theirs_and_checked(store: &dyn Store) {
    assert!(store.set_receiver_name(Some("Μαρία")).is_err());
    store.create_device(&device()).unwrap();
    assert_eq!(store.receiver_name().unwrap(), None);
    store.set_receiver_name(Some("Μαρία")).unwrap();
    assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    for bad in ["", "  ", "a\u{1b}", &"x".repeat(41)] {
        assert!(store.set_receiver_name(Some(bad)).is_err(), "{bad:?}");
    }
    assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    store.set_receiver_name(None).unwrap();
    assert_eq!(store.receiver_name().unwrap(), None);
}

fn a_counter_grows_even_when_the_clock_goes_back(store: &dyn Store) {
    let stick = JoinedStick {
        kind: StickKind::App,
        ..joined(1, 10)
    };
    let id = stick.stick_id;
    assert!(matches!(
        store.next_report_counter(&id, NOW),
        Err(StoreError::StickNotFound)
    ));
    store.add_joined_stick(&stick).unwrap();
    assert_eq!(store.next_report_counter(&id, NOW).unwrap(), NOW);
    assert_eq!(store.next_report_counter(&id, NOW).unwrap(), NOW + 1);
    // The clock set back: still above the last.
    assert_eq!(
        store.next_report_counter(&id, NOW - 60_000).unwrap(),
        NOW + 2
    );
    assert_eq!(store.next_report_counter(&id, NOW + 50).unwrap(), NOW + 50);
    // Being forgotten: no more reports.
    store
        .change_joined_stick(&id, &crate::store::JoinedChange::Forgetting)
        .unwrap();
    assert!(store.next_report_counter(&id, NOW).is_err());
}

for_every_store!(
    a_readers_latest_report_is_kept_and_an_old_one_changes_nothing,
    a_removed_line_comes_back_only_with_a_newer_report,
    a_flood_fills_only_the_free_places,
    the_owners_clock_set_back_changes_nothing_kept,
    lines_older_than_ninety_days_are_not_shown_and_go,
    reports_are_only_for_sticks_shared_here_and_go_with_them,
    the_receivers_name_is_theirs_and_checked,
    a_counter_grows_even_when_the_clock_goes_back,
);
