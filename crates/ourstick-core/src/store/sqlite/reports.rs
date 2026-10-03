//! Readers' reports on owned sticks (`reader_reports`), the receiver's name
//! for senders (`device.receiver_name`), and a received stick's report
//! counter (`joined_sticks.report_counter`), ADR 0015.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::super::reports::{
    check_receiver_name, check_report, kept_since, next_counter, room_for, Held, KeepOutcome,
    KeptReport, Room,
};
use super::super::StoreError;
use super::rows::{blob32, seq_i64, time_i64, to_u64};
use crate::report::{ComputerKind, ReaderId, ReaderReport, ReportState, WaitReason};
use crate::stick_id::StickId;

pub(super) fn keep_report(
    connection: &mut Connection,
    stick_id: &StickId,
    reader: &ReaderId,
    report: &ReaderReport,
    now_ms: u64,
) -> Result<KeepOutcome, StoreError> {
    check_report(report, now_ms)?;
    let id = stick_id.to_bytes();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let live: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM owned_sticks WHERE stick_id = ?1 AND removing = 0)",
        [id],
        |row| row.get(0),
    )?;
    if !live {
        return Err(StoreError::StickNotFound);
    }
    transaction.execute(
        "DELETE FROM reader_reports WHERE stick_id = ?1 AND received_ms < ?2",
        params![id, time_i64(kept_since(now_ms))?],
    )?;
    let forgotten = report.state == ReportState::Forgotten;
    let known: Option<(i64, i64)> = transaction
        .query_row(
            "SELECT counter, first_ms FROM reader_reports WHERE stick_id = ?1 AND reader_id = ?2",
            params![id, reader.to_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let first_ms = match known {
        Some((counter, _)) if to_u64(counter, "counter")? >= report.counter => {
            return Ok(KeepOutcome::Old)
        }
        Some((_, first_ms)) => to_u64(first_ms, "first_ms")?,
        None if forgotten => return Ok(KeepOutcome::Ignored),
        None => {
            match room_for(&held(&transaction, &id)?, now_ms) {
                Room::Free => {}
                Room::Instead(gone) => {
                    transaction.execute(
                        "DELETE FROM reader_reports WHERE stick_id = ?1 AND reader_id = ?2",
                        params![id, gone.to_bytes()],
                    )?;
                }
                Room::Full => return Ok(KeepOutcome::Full),
            }
            now_ms
        }
    };
    let kept = KeptReport::new(*reader, report, first_ms, now_ms);
    let (state, percent, reason) = state_columns(kept.state);
    transaction.execute(
        "INSERT OR REPLACE INTO reader_reports
         (stick_id, reader_id, counter, seq, state, percent, reason, everything, kind, name,
          first_ms, received_ms, hidden)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id,
            reader.to_bytes(),
            seq_i64(kept.counter)?,
            seq_i64(kept.seq)?,
            state,
            percent,
            reason,
            kept.everything,
            kind_text(kept.kind),
            kept.name,
            time_i64(first_ms)?,
            time_i64(now_ms)?,
            forgotten,
        ],
    )?;
    transaction.commit()?;
    Ok(KeepOutcome::Kept)
}

fn held(connection: &Connection, id: &[u8; 32]) -> Result<Vec<Held>, StoreError> {
    let mut statement = connection
        .prepare("SELECT reader_id, received_ms, hidden FROM reader_reports WHERE stick_id = ?1")?;
    let rows = statement.query_map([id], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, bool>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (reader, received_ms, hidden) = row?;
        Ok(Held {
            reader: ReaderId::from_bytes(blob32(reader, "reader_id")?),
            received_ms: to_u64(received_ms, "received_ms")?,
            hidden,
        })
    })
    .collect()
}

pub(super) fn reports(
    connection: &Connection,
    stick_id: &StickId,
    now_ms: u64,
) -> Result<Vec<KeptReport>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT reader_id, counter, seq, state, percent, reason, everything, kind, name,
                first_ms, received_ms
         FROM reader_reports
         WHERE stick_id = ?1 AND hidden = 0 AND received_ms >= ?2
         ORDER BY received_ms DESC, reader_id",
    )?;
    let rows = statement.query_map(
        params![stick_id.to_bytes(), time_i64(kept_since(now_ms))?],
        |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                [row.get::<_, i64>(1)?, row.get::<_, i64>(2)?],
                (
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ),
                row.get::<_, bool>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                [row.get::<_, i64>(9)?, row.get::<_, i64>(10)?],
            ))
        },
    )?;
    rows.map(|row| {
        let (reader, [counter, seq], state, everything, kind, name, [first_ms, received_ms]) = row?;
        Ok(KeptReport {
            reader: ReaderId::from_bytes(blob32(reader, "reader_id")?),
            counter: to_u64(counter, "counter")?,
            seq: to_u64(seq, "seq")?,
            state: state_from(state)?,
            everything,
            kind: kind_from(&kind)?,
            name,
            first_ms: to_u64(first_ms, "first_ms")?,
            received_ms: to_u64(received_ms, "received_ms")?,
        })
    })
    .collect()
}

pub(super) fn hide(
    connection: &Connection,
    stick_id: &StickId,
    reader: Option<&ReaderId>,
) -> Result<(), StoreError> {
    match reader {
        Some(reader) => connection.execute(
            "UPDATE reader_reports SET hidden = 1 WHERE stick_id = ?1 AND reader_id = ?2",
            params![stick_id.to_bytes(), reader.to_bytes()],
        )?,
        None => connection.execute(
            "UPDATE reader_reports SET hidden = 1 WHERE stick_id = ?1",
            [stick_id.to_bytes()],
        )?,
    };
    Ok(())
}

pub(super) fn receiver_name(connection: &Connection) -> Result<Option<String>, StoreError> {
    Ok(connection
        .query_row("SELECT receiver_name FROM device", [], |row| row.get(0))
        .optional()?
        .flatten())
}

pub(super) fn set_receiver_name(
    connection: &Connection,
    name: Option<&str>,
) -> Result<(), StoreError> {
    check_receiver_name(name)?;
    let updated = connection.execute("UPDATE device SET receiver_name = ?1", [name])?;
    if updated == 0 {
        return Err(StoreError::Inconsistent("this device is not set up yet"));
    }
    Ok(())
}

pub(super) fn next_report_counter(
    connection: &mut Connection,
    stick_id: &StickId,
    now_ms: u64,
) -> Result<u64, StoreError> {
    time_i64(now_ms)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last: Option<i64> = transaction
        .query_row(
            "SELECT report_counter FROM joined_sticks WHERE stick_id = ?1 AND forgetting = 0",
            [stick_id.to_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(last) = last else {
        return Err(StoreError::StickNotFound);
    };
    let next = next_counter(to_u64(last, "report_counter")?, now_ms);
    transaction.execute(
        "UPDATE joined_sticks SET report_counter = ?1 WHERE stick_id = ?2",
        params![seq_i64(next)?, stick_id.to_bytes()],
    )?;
    transaction.commit()?;
    Ok(next)
}

fn state_columns(state: ReportState) -> (&'static str, Option<u8>, Option<&'static str>) {
    match state {
        ReportState::Downloading { percent } => ("downloading", Some(percent), None),
        ReportState::UpToDate => ("up_to_date", None, None),
        ReportState::Waiting(reason) => ("waiting", None, Some(reason_text(reason))),
        ReportState::Paused => ("paused", None, None),
        ReportState::Forgotten => ("forgotten", None, None),
    }
}

fn state_from(
    (state, percent, reason): (String, Option<i64>, Option<String>),
) -> Result<ReportState, StoreError> {
    let corrupt = || StoreError::Corrupt(format!("report state {state}"));
    Ok(match (state.as_str(), percent, reason.as_deref()) {
        ("downloading", Some(percent), None) => ReportState::Downloading {
            percent: u8::try_from(percent)
                .ok()
                .filter(|percent| *percent <= 99)
                .ok_or_else(corrupt)?,
        },
        ("up_to_date", None, None) => ReportState::UpToDate,
        ("waiting", None, Some(reason)) => ReportState::Waiting(reason_from(reason)?),
        ("paused", None, None) => ReportState::Paused,
        ("forgotten", None, None) => ReportState::Forgotten,
        _ => return Err(corrupt()),
    })
}

fn reason_text(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::SenderAway => "sender_away",
        WaitReason::NoRoom => "no_room",
        WaitReason::FolderMissing => "folder_missing",
        WaitReason::Other => "other",
    }
}

fn reason_from(text: &str) -> Result<WaitReason, StoreError> {
    Ok(match text {
        "sender_away" => WaitReason::SenderAway,
        "no_room" => WaitReason::NoRoom,
        "folder_missing" => WaitReason::FolderMissing,
        "other" => WaitReason::Other,
        _ => return Err(StoreError::Corrupt(format!("wait reason {text}"))),
    })
}

fn kind_text(kind: ComputerKind) -> &'static str {
    match kind {
        ComputerKind::MacOs => "macos",
        ComputerKind::Windows => "windows",
        ComputerKind::Linux => "linux",
        ComputerKind::Other => "other",
    }
}

fn kind_from(text: &str) -> Result<ComputerKind, StoreError> {
    Ok(match text {
        "macos" => ComputerKind::MacOs,
        "windows" => ComputerKind::Windows,
        "linux" => ComputerKind::Linux,
        "other" => ComputerKind::Other,
        _ => return Err(StoreError::Corrupt(format!("computer kind {text}"))),
    })
}
