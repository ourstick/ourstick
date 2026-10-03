//! What an owner keeps of its readers' reports (ADR 0015, rule 5), and the
//! rules both stores keep it by.

use super::{check_seq, check_time, StoreError};
use crate::report::{name_is_allowed, ComputerKind, ReaderId, ReaderReport, ReportState};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The most readers kept per stick.
pub const MAX_READERS: usize = 200;

/// A reader silent this long may make room for a new one.
pub const SILENT_MS: u64 = 30 * DAY_MS;

/// A line older than this is dropped, and never shown.
pub const KEPT_MS: u64 = 90 * DAY_MS;

/// A reader's latest report, as the owner keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptReport {
    pub reader: ReaderId,
    pub counter: u64,
    pub seq: u64,
    pub state: ReportState,
    pub everything: bool,
    pub kind: ComputerKind,
    pub name: Option<String>,
    /// When the owner first heard from this reader (its own clock).
    pub first_ms: u64,
    /// When the owner received this report (its own clock).
    pub received_ms: u64,
}

impl KeptReport {
    pub(super) fn new(reader: ReaderId, report: &ReaderReport, first_ms: u64, now_ms: u64) -> Self {
        Self {
            reader,
            counter: report.counter,
            seq: report.seq,
            state: report.state,
            everything: report.everything,
            kind: report.kind,
            name: report.name.clone(),
            first_ms,
            received_ms: now_ms,
        }
    }
}

/// What became of a report the owner was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOutcome {
    /// Its line shows it now (or went, for `Forgotten`).
    Kept,
    /// Its counter is not above the last one kept for its reader: an old
    /// report sent again, or one overtaken.
    Old,
    /// A new reader, and the stick already keeps [`MAX_READERS`] that have
    /// all spoken in the last [`SILENT_MS`] and none hidden.
    Full,
    /// `Forgotten` from a reader never kept: nothing to take off.
    Ignored,
}

/// One kept reader, as [`room_for`] weighs it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Held {
    pub reader: ReaderId,
    pub received_ms: u64,
    pub hidden: bool,
}

/// Where a new reader goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Room {
    Free,
    /// In place of this reader: a hidden one first, else the longest
    /// silent, if silent for [`SILENT_MS`].
    Instead(ReaderId),
    Full,
}

/// The one place that decides whether a new reader is kept, so a flood of
/// made-up readers fills only the free places and never pushes off a
/// reader that still speaks.
pub(super) fn room_for(held: &[Held], now_ms: u64) -> Room {
    if held.len() < MAX_READERS {
        return Room::Free;
    }
    let oldest = |hidden: bool| {
        held.iter()
            .filter(|kept| kept.hidden == hidden)
            .min_by_key(|kept| (kept.received_ms, kept.reader))
    };
    if let Some(kept) = oldest(true) {
        return Room::Instead(kept.reader);
    }
    match oldest(false) {
        Some(kept) if now_ms.saturating_sub(kept.received_ms) >= SILENT_MS => {
            Room::Instead(kept.reader)
        }
        _ => Room::Full,
    }
}

/// Lines received before this are dropped.
pub(super) fn kept_since(now_ms: u64) -> u64 {
    now_ms.saturating_sub(KEPT_MS)
}

/// What a store keeps: what `report::open` already checked, checked again
/// at the door, and numbers the database can hold.
pub(super) fn check_report(report: &ReaderReport, now_ms: u64) -> Result<(), StoreError> {
    check_seq(report.seq)?;
    check_seq(report.counter)?;
    check_time(now_ms)?;
    if let ReportState::Downloading { percent } = report.state {
        if percent > 99 {
            return Err(StoreError::Inconsistent("a percent is below 100"));
        }
    }
    if report
        .name
        .as_deref()
        .is_some_and(|name| !name_is_allowed(name))
    {
        return Err(StoreError::Inconsistent("a reader's name breaks the rules"));
    }
    Ok(())
}

/// A name for senders, as the receiver gives it: checked like a report's.
pub(super) fn check_receiver_name(name: Option<&str>) -> Result<(), StoreError> {
    match name {
        Some(name) if !name_is_allowed(name) => Err(StoreError::Inconsistent(
            "a name for senders is 1-40 characters, none invisible",
        )),
        _ => Ok(()),
    }
}

/// The counter of a reader's next report: above the last, and the clock's
/// time when the clock is ahead.
pub(super) fn next_counter(last: u64, now_ms: u64) -> u64 {
    now_ms.max(last.saturating_add(1))
}
