//! [`MemoryStore`]'s stick life: rename and settings, removal, new links.

use super::MemoryStore;
use crate::stick_id::StickId;
use crate::store::{check_owned_stick, check_time, OwnedStick, StoreError};

/// The live sticks, or (`removing`) the ones being removed, in order.
pub(super) fn listed(store: &MemoryStore, removing: bool) -> Vec<OwnedStick> {
    let state = store.state();
    super::sorted(
        state
            .owned
            .iter()
            .filter(|(key, _)| state.removing.contains(*key) == removing)
            .map(|(key, stick)| (stick.created_at_ms, *key, stick.clone())),
    )
}

/// Whether `key` is a stick of this store that is not being removed.
pub(super) fn is_live(state: &super::State, key: &[u8; 32]) -> bool {
    state.owned.contains_key(key) && !state.removing.contains(key)
}

pub(super) fn save_signed_record(
    store: &MemoryStore,
    record: &crate::record::SignedManifestRecord,
    partial: bool,
) -> Result<(), StoreError> {
    let content = record.unverified_record();
    crate::store::check_seq(content.seq)?;
    let mut state = store.state();
    let key = content.stick_id.to_bytes();
    if !is_live(&state, &key) {
        return Err(StoreError::StickNotFound);
    }
    let stick = state.owned.get_mut(&key).ok_or(StoreError::StickNotFound)?;
    if content.seq <= stick.last_signed_seq {
        return Err(StoreError::NotNewer {
            seq: content.seq,
            stored: stick.last_signed_seq,
        });
    }
    stick.last_signed_seq = content.seq;
    stick.latest_record = Some(record.clone());
    stick.latest_partial = partial;
    Ok(())
}

/// Changes the live (not removing) stick `stick_id`.
pub(super) fn change(
    store: &MemoryStore,
    stick_id: &StickId,
    edit: impl FnOnce(&mut OwnedStick),
) -> Result<(), StoreError> {
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if state.removing.contains(&key) {
        return Err(StoreError::StickNotFound);
    }
    let stick = state.owned.get_mut(&key).ok_or(StoreError::StickNotFound)?;
    edit(stick);
    Ok(())
}

pub(super) fn mark_removing(store: &MemoryStore, stick_id: &StickId) -> Result<(), StoreError> {
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !state.owned.contains_key(&key) || state.removing.contains(&key) {
        return Err(StoreError::StickNotFound);
    }
    state.removing.insert(key);
    Ok(())
}

/// Every row of `key` but the stick's own.
fn forget_stick_rows(state: &mut super::State, key: &[u8; 32]) {
    state.owned_files.remove(key);
    state.linked.remove(key);
    state.claims.retain(|(stick, _), _| stick != key);
}

pub(super) fn remove(store: &MemoryStore, stick_id: &StickId) -> Result<(), StoreError> {
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !state.owned.contains_key(&key) {
        return Err(StoreError::StickNotFound);
    }
    if !state.removing.contains(&key) {
        return Err(StoreError::Inconsistent(
            "only a stick marked as being removed is deleted",
        ));
    }
    forget_stick_rows(&mut state, &key);
    state.owned.remove(&key);
    state.removing.remove(&key);
    Ok(())
}

pub(super) fn replace(
    store: &MemoryStore,
    old: &StickId,
    new: &OwnedStick,
    now_ms: u64,
) -> Result<(), StoreError> {
    check_owned_stick(new)?;
    check_time(now_ms)?;
    if new.stick_id == *old {
        return Err(StoreError::Inconsistent("a new link is a new stick id"));
    }
    let mut state = store.state();
    let old_key = old.to_bytes();
    let new_key = new.stick_id.to_bytes();
    if !state.owned.contains_key(&old_key) || state.removing.contains(&old_key) {
        return Err(StoreError::StickNotFound);
    }
    let same_node = new.node_id.is_some()
        && state
            .owned
            .values()
            .any(|owned| owned.node_id == new.node_id);
    if state.owned.contains_key(&new_key) || same_node {
        return Err(StoreError::StickExists);
    }
    state.owned.insert(new_key, new.clone());
    if let Some(linked) = state.linked.remove(&old_key) {
        state.linked.insert(new_key, linked);
    }
    forget_stick_rows(&mut state, &old_key);
    state.owned.remove(&old_key);
    state.retired.insert(old_key, now_ms);
    Ok(())
}

pub(super) fn retired(store: &MemoryStore) -> Result<Vec<StickId>, StoreError> {
    let state = store.state();
    let mut retired: Vec<(u64, [u8; 32])> =
        state.retired.iter().map(|(key, at)| (*at, *key)).collect();
    retired.sort();
    retired
        .into_iter()
        .map(|(_, key)| {
            StickId::from_bytes(&key)
                .map_err(|_| StoreError::Corrupt("a retired id is not a key".to_owned()))
        })
        .collect()
}
