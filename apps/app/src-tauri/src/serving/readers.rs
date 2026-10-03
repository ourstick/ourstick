//! Who has a shared stick (ADR 0015): readers' reports kept as they come,
//! and the lines the window shows under the stick.

use std::sync::Arc;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::report::{ComputerKind, ReaderId, WaitReason};
use ourstick_core::share::{has, keep_reports, latest_hint, Has};
use ourstick_core::store::Store;
use ourstick_core::untrusted::escape;
use ourstick_core::{StickId, StickKey};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::events::StickEvent;
use crate::state::{stick_id as stick_id_of, AppState};

/// Keeps `stick_id`'s readers' reports from now on, telling the window
/// (`readers-changed`) when its list changed. Ends with the publisher.
pub(super) fn keep(
    app: &AppHandle,
    reports: tokio::sync::mpsc::Receiver<Vec<u8>>,
    store: Arc<dyn Store>,
    stick_id: StickId,
    stick_key: StickKey,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        keep_reports(
            reports,
            store,
            stick_id,
            stick_key,
            Arc::new(SystemClock),
            || {
                let _ = app.emit("readers-changed", StickEvent::new(&stick_id, ()));
            },
        )
        .await;
    });
}

/// One computer that received the stick, as its line under the stick says
/// it; the window makes the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReaderLine {
    /// Its reader id, for "Remove from this list".
    reader: String,
    /// "macos", "windows", "linux", "other".
    kind: &'static str,
    /// A name its receiver typed, escaped; none usually.
    name: Option<String>,
    /// "all", "what_it_chose", "so_far", "older", "downloading", "waiting",
    /// "paused".
    has: &'static str,
    everything: bool,
    percent: Option<u8>,
    /// "sender_away", "no_room", "folder_missing", "other" while waiting.
    reason: Option<&'static str>,
    /// When this computer first heard from it, and when last (its own
    /// clock).
    first_ms: u64,
    received_ms: u64,
}

/// The lines under a shared stick, the latest first.
pub fn reader_lines(
    store: &dyn Store,
    stick_id: &StickId,
    now_ms: u64,
) -> anyhow::Result<Vec<ReaderLine>> {
    let latest = latest_hint(store, stick_id)?.map(|hint| {
        let partial = store
            .owned_stick(stick_id)
            .ok()
            .flatten()
            .is_some_and(|stick| stick.latest_partial);
        (hint.seq, partial)
    });
    let mut lines = Vec::new();
    for kept in store.reports(stick_id, now_ms)? {
        let Some(said) = has(&kept, latest) else {
            continue;
        };
        let (has, everything, percent, reason) = match said {
            Has::All => ("all", true, None, None),
            Has::WhatItChose => ("what_it_chose", false, None, None),
            Has::SoFar { everything } => ("so_far", everything, None, None),
            Has::Older { everything } => ("older", everything, None, None),
            Has::Downloading { percent } => ("downloading", kept.everything, Some(percent), None),
            Has::Waiting(reason) => ("waiting", kept.everything, None, Some(reason_word(reason))),
            Has::Paused => ("paused", kept.everything, None, None),
        };
        lines.push(ReaderLine {
            reader: kept
                .reader
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            kind: kind_word(kept.kind),
            name: kept.name.as_deref().map(escape),
            has,
            everything,
            percent,
            reason,
            first_ms: kept.first_ms,
            received_ms: kept.received_ms,
        });
    }
    Ok(lines)
}

fn kind_word(kind: ComputerKind) -> &'static str {
    match kind {
        ComputerKind::MacOs => "macos",
        ComputerKind::Windows => "windows",
        ComputerKind::Linux => "linux",
        ComputerKind::Other => "other",
    }
}

fn reason_word(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::SenderAway => "sender_away",
        WaitReason::NoRoom => "no_room",
        WaitReason::FolderMissing => "folder_missing",
        WaitReason::Other => "other",
    }
}

fn reader_of(hex: &str) -> Result<ReaderId, String> {
    let gone = || "That line is not there any more.".to_owned();
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(gone());
    }
    let mut bytes = [0u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| gone())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| gone())?;
    }
    Ok(ReaderId::from_bytes(bytes))
}

/// The lines under a shared stick.
#[tauri::command]
pub async fn readers(
    stick_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ReaderLine>, String> {
    let stick_id = stick_id_of(&stick_id)?;
    let store = state.store.clone();
    crate::receiving::work(move || reader_lines(store.as_ref(), &stick_id, SystemClock.now_ms()))
        .await
}

/// "Remove from this list": one line, or every line (`reader` none).
#[tauri::command]
pub async fn remove_readers(
    stick_id: String,
    reader: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    let reader = reader.as_deref().map(reader_of).transpose()?;
    let store = state.store.clone();
    crate::receiving::work(move || {
        match reader {
            Some(reader) => store.hide_report(&stick_id, &reader)?,
            None => store.hide_reports(&stick_id)?,
        }
        Ok(())
    })
    .await?;
    let _ = app.emit("readers-changed", StickEvent::new(&stick_id, ()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::report::{ReaderReport, ReportState};
    use ourstick_core::store::{MemoryStore, OwnedStick};

    #[test]
    fn the_lines_say_what_each_computer_has() {
        let store = MemoryStore::new();
        let id = ourstick_core::StickSigningKey::from_bytes(&[3; 32]).stick_id();
        store
            .add_owned_stick(&OwnedStick::new(
                id,
                "Holiday".to_owned(),
                "/s",
                ourstick_core::cipher::CipherId::XChaCha20Poly1305,
                1,
            ))
            .unwrap();
        let report = |counter, state, name: Option<&str>| ReaderReport {
            counter,
            seq: 0,
            state,
            everything: false,
            kind: ComputerKind::MacOs,
            name: name.map(str::to_owned),
        };
        let one = ReaderId::from_bytes([1; 32]);
        let two = ReaderId::from_bytes([2; 32]);
        store
            .keep_report(
                &id,
                &one,
                &report(1, ReportState::Paused, Some("Μαρία")),
                10,
            )
            .unwrap();
        store
            .keep_report(
                &id,
                &two,
                &report(1, ReportState::Downloading { percent: 40 }, None),
                20,
            )
            .unwrap();
        let lines = reader_lines(&store, &id, 30).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].has, lines[0].percent), ("downloading", Some(40)));
        assert_eq!(lines[1].has, "paused");
        assert_eq!(lines[1].name.as_deref(), Some("Μαρία"));
        assert_eq!(lines[1].kind, "macos");
        assert_eq!(reader_of(&lines[1].reader).unwrap(), one);
        assert!(reader_of("zz").is_err());
        // Up to date with a version the owner never published: an older one.
        store
            .keep_report(&id, &two, &report(2, ReportState::UpToDate, None), 40)
            .unwrap();
        assert_eq!(reader_lines(&store, &id, 50).unwrap()[0].has, "older");
    }
}
