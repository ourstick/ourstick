//! Received (joined) sticks in memory: as the SQLite store keeps them, those
//! being forgotten hidden from the usual reads.

use std::path::Path;

use super::{sorted, MemoryStore};
use crate::stick_id::StickId;
use crate::store::{
    check_joined_stick, check_seq, path_str, ClaimKind, JoinedChange, JoinedStick, ManifestVersion,
    StoreError,
};

pub(super) fn add(store: &MemoryStore, stick: &JoinedStick) -> Result<(), StoreError> {
    check_joined_stick(stick)?;
    let mut state = store.state();
    let key = stick.stick_id.to_bytes();
    if state.joined.contains_key(&key) {
        return Err(StoreError::StickExists);
    }
    state.joined.insert(key, stick.clone());
    Ok(())
}

pub(super) fn get(
    store: &MemoryStore,
    stick_id: &StickId,
) -> Result<Option<JoinedStick>, StoreError> {
    let state = store.state();
    let key = stick_id.to_bytes();
    if state.forgetting.contains(&key) {
        return Ok(None);
    }
    Ok(state.joined.get(&key).cloned())
}

/// The live sticks, or (`forgetting`) those being forgotten, oldest first.
pub(super) fn list(store: &MemoryStore, forgetting: bool) -> Result<Vec<JoinedStick>, StoreError> {
    let state = store.state();
    Ok(sorted(
        state
            .joined
            .iter()
            .filter(|(key, _)| state.forgetting.contains(*key) == forgetting)
            .map(|(key, stick)| (stick.joined_at_ms, *key, stick.clone())),
    ))
}

/// The live stick `stick_id`, to change.
fn live<'a>(
    joined: &'a mut std::collections::BTreeMap<[u8; 32], JoinedStick>,
    forgetting: &std::collections::BTreeSet<[u8; 32]>,
    stick_id: &StickId,
) -> Result<&'a mut JoinedStick, StoreError> {
    let key = stick_id.to_bytes();
    if forgetting.contains(&key) {
        return Err(StoreError::StickNotFound);
    }
    joined.get_mut(&key).ok_or(StoreError::StickNotFound)
}

pub(super) fn move_to(
    store: &MemoryStore,
    stick_id: &StickId,
    output_dir: &Path,
) -> Result<(), StoreError> {
    path_str(output_dir)?;
    let mut state = store.state();
    let state = &mut *state;
    let stick = live(&mut state.joined, &state.forgetting, stick_id)?;
    stick.output_dir = output_dir.to_path_buf();
    stick.completed = None;
    state.downloaded_files.remove(&stick_id.to_bytes());
    Ok(())
}

pub(super) fn complete(
    store: &MemoryStore,
    stick_id: &StickId,
    version: ManifestVersion,
) -> Result<(), StoreError> {
    check_seq(version.seq)?;
    let mut state = store.state();
    let state = &mut *state;
    if state
        .floors
        .get(&stick_id.to_bytes())
        .is_none_or(|floor| floor.seq < version.seq)
    {
        return Err(StoreError::Inconsistent(
            "a download completed for a record that was never accepted",
        ));
    }
    live(&mut state.joined, &state.forgetting, stick_id)?.completed = Some(version);
    Ok(())
}

pub(super) fn change(
    store: &MemoryStore,
    stick_id: &StickId,
    change: &JoinedChange,
) -> Result<(), StoreError> {
    let mut state = store.state();
    let state = &mut *state;
    let stick = live(&mut state.joined, &state.forgetting, stick_id)?;
    match change {
        JoinedChange::Name(name) => stick.name = Some(name.clone()),
        JoinedChange::Kind(kind) => stick.kind = *kind,
        JoinedChange::Paused(paused) => stick.paused = *paused,
        JoinedChange::Forgetting => {
            state.forgetting.insert(stick_id.to_bytes());
        }
    }
    Ok(())
}

pub(super) fn forget(store: &MemoryStore, stick_id: &StickId) -> Result<(), StoreError> {
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !state.forgetting.remove(&key) {
        return Err(StoreError::Inconsistent(
            "only a received stick marked as being forgotten is forgotten",
        ));
    }
    // The record floor stays (ADR 0004).
    state.joined.remove(&key);
    state.downloaded_files.remove(&key);
    for kind in [ClaimKind::Download, ClaimKind::Scratch] {
        state.claims.remove(&super::claim_key(stick_id, kind));
    }
    Ok(())
}
