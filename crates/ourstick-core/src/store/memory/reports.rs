//! Readers' reports, the receiver's name and report counters, as the
//! database keeps them (ADR 0015); the keys' move (ADR 0021).

use super::MemoryStore;
use crate::report::{ReaderId, ReaderReport, ReportState};
use crate::stick_id::StickId;
use crate::store::reports::{
    check_receiver_name, check_report, kept_since, next_counter, room_for, Held, KeepOutcome,
    KeptReport, Room,
};
use crate::store::StoreError;

/// A kept line and whether the sender's list hides it.
#[derive(Debug, Clone)]
pub(super) struct Line {
    kept: KeptReport,
    hidden: bool,
}

pub(super) fn keep(
    store: &MemoryStore,
    stick_id: &StickId,
    reader: &ReaderId,
    report: &ReaderReport,
    now_ms: u64,
) -> Result<KeepOutcome, StoreError> {
    check_report(report, now_ms)?;
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !state.owned.contains_key(&key) || state.removing.contains(&key) {
        return Err(StoreError::StickNotFound);
    }
    let lines = state.reports.entry(key).or_default();
    let since = kept_since(now_ms);
    lines.retain(|_, line| line.kept.received_ms >= since);
    let forgotten = report.state == ReportState::Forgotten;
    let first_ms = match lines.get(reader) {
        Some(line) if line.kept.counter >= report.counter => return Ok(KeepOutcome::Old),
        Some(line) => line.kept.first_ms,
        None if forgotten => return Ok(KeepOutcome::Ignored),
        None => {
            let held: Vec<Held> = lines
                .values()
                .map(|line| Held {
                    reader: line.kept.reader,
                    received_ms: line.kept.received_ms,
                    hidden: line.hidden,
                })
                .collect();
            match room_for(&held, now_ms) {
                Room::Free => {}
                Room::Instead(gone) => {
                    lines.remove(&gone);
                }
                Room::Full => return Ok(KeepOutcome::Full),
            }
            now_ms
        }
    };
    lines.insert(
        *reader,
        Line {
            kept: KeptReport::new(*reader, report, first_ms, now_ms),
            hidden: forgotten,
        },
    );
    Ok(KeepOutcome::Kept)
}

pub(super) fn reports(
    store: &MemoryStore,
    stick_id: &StickId,
    now_ms: u64,
) -> Result<Vec<KeptReport>, StoreError> {
    let since = kept_since(now_ms);
    let mut shown: Vec<KeptReport> = store
        .state()
        .reports
        .get(&stick_id.to_bytes())
        .map(|lines| {
            lines
                .values()
                .filter(|line| !line.hidden && line.kept.received_ms >= since)
                .map(|line| line.kept.clone())
                .collect()
        })
        .unwrap_or_default();
    shown.sort_by(|a, b| {
        b.received_ms
            .cmp(&a.received_ms)
            .then_with(|| a.reader.to_bytes().cmp(&b.reader.to_bytes()))
    });
    Ok(shown)
}

pub(super) fn hide(
    store: &MemoryStore,
    stick_id: &StickId,
    reader: Option<&ReaderId>,
) -> Result<(), StoreError> {
    if let Some(lines) = store.state().reports.get_mut(&stick_id.to_bytes()) {
        for (id, line) in lines.iter_mut() {
            if reader.is_none_or(|reader| reader == id) {
                line.hidden = true;
            }
        }
    }
    Ok(())
}

pub(super) fn set_receiver_name(store: &MemoryStore, name: Option<&str>) -> Result<(), StoreError> {
    check_receiver_name(name)?;
    let mut state = store.state();
    if state.device.is_none() {
        return Err(StoreError::Inconsistent("this device is not set up yet"));
    }
    state.receiver_name = name.map(str::to_owned);
    Ok(())
}

pub(super) fn next_report_counter(
    store: &MemoryStore,
    stick_id: &StickId,
    now_ms: u64,
) -> Result<u64, StoreError> {
    crate::store::check_time(now_ms)?;
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !state.joined.contains_key(&key) || state.forgetting.contains(&key) {
        return Err(StoreError::StickNotFound);
    }
    let last = state.report_counters.get(&key).copied().unwrap_or(0);
    let next = next_counter(last, now_ms);
    crate::store::check_seq(next)?;
    state.report_counters.insert(key, next);
    Ok(next)
}

pub(super) fn record_keys_moved(
    store: &MemoryStore,
    old_keys_left: bool,
) -> Result<(), StoreError> {
    use crate::store::SecretBackend;
    let mut state = store.state();
    let Some(device) = state.device.as_mut() else {
        return Err(StoreError::Inconsistent("this device is not set up yet"));
    };
    if !matches!(
        device.secret_backend,
        SecretBackend::Keychain | SecretBackend::ProtectedKeychain
    ) {
        return Err(StoreError::Inconsistent(
            "the device's keys are not in the macOS keychain",
        ));
    }
    device.secret_backend = SecretBackend::ProtectedKeychain;
    state.old_keys_left = old_keys_left;
    Ok(())
}
