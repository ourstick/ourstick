//! [`MemoryStore`]'s linked entries and space claims.

use super::{claim_key, MemoryStore};
use crate::stick_id::StickId;
use crate::store::{
    check_at_top, check_linked_entry, check_value, ClaimKind, LinkedEntry, SpaceClaim, StoreError,
    AT_TOP_STAYS,
};

pub(super) fn add_linked_entry(
    store: &MemoryStore,
    stick_id: &StickId,
    entry: &LinkedEntry,
) -> Result<(), StoreError> {
    check_linked_entry(entry)?;
    let mut state = store.state();
    let key = stick_id.to_bytes();
    if !super::life::is_live(&state, &key) {
        return Err(StoreError::StickNotFound);
    }
    if entry.at_top {
        let has_own_folder = state
            .owned
            .get(&key)
            .is_some_and(|stick| stick.has_own_folder);
        let one_there = state
            .linked
            .get(&key)
            .is_some_and(|entries| entries.values().any(|other| other.at_top));
        check_at_top(has_own_folder, one_there)?;
    }
    let entries = state.linked.entry(key).or_default();
    if entries.contains_key(&entry.name) {
        return Err(StoreError::LinkExists(entry.name.clone()));
    }
    entries.insert(entry.name.clone(), entry.clone());
    Ok(())
}

pub(super) fn linked_entries(
    store: &MemoryStore,
    stick_id: &StickId,
) -> Result<Vec<LinkedEntry>, StoreError> {
    Ok(store
        .state()
        .linked
        .get(&stick_id.to_bytes())
        .map(|entries| entries.values().cloned().collect())
        .unwrap_or_default())
}

pub(super) fn remove_linked_entry(
    store: &MemoryStore,
    stick_id: &StickId,
    name: &str,
) -> Result<(), StoreError> {
    let mut state = store.state();
    let key = stick_id.to_bytes();
    let entries = state.linked.get_mut(&key);
    match entries.as_ref().and_then(|entries| entries.get(name)) {
        None => return Err(StoreError::LinkNotFound(name.to_owned())),
        Some(entry) if entry.at_top => return Err(StoreError::Inconsistent(AT_TOP_STAYS)),
        Some(_) => {}
    }
    entries.map(|entries| entries.remove(name));
    let prefix = format!("{name}/");
    if let Some(files) = state.owned_files.get_mut(&key) {
        files.retain(|path, _| path != name && !path.starts_with(&prefix));
    }
    Ok(())
}

pub(super) fn space_claims(store: &MemoryStore) -> Result<Vec<SpaceClaim>, StoreError> {
    Ok(store.state().claims.values().cloned().collect())
}

pub(super) fn claim_space(
    store: &MemoryStore,
    decide: &mut dyn FnMut(&[SpaceClaim]) -> Vec<SpaceClaim>,
) -> Result<Vec<SpaceClaim>, StoreError> {
    let mut state = store.state();
    let all: Vec<SpaceClaim> = state.claims.values().cloned().collect();
    let decided = decide(&all);
    // All or nothing, as the SQLite transaction.
    for claim in &decided {
        check_value("bytes", claim.bytes)?;
        if claim.disk.is_empty() {
            return Err(StoreError::Inconsistent("a claim names its disk"));
        }
    }
    for claim in &decided {
        state
            .claims
            .insert(claim_key(&claim.stick_id, claim.kind), claim.clone());
    }
    Ok(decided)
}

pub(super) fn update_space_claim(
    store: &MemoryStore,
    stick_id: &StickId,
    kind: ClaimKind,
    bytes: u64,
) -> Result<(), StoreError> {
    check_value("bytes", bytes)?;
    if let Some(claim) = store.state().claims.get_mut(&claim_key(stick_id, kind)) {
        claim.bytes = bytes;
    }
    Ok(())
}

pub(super) fn remove_space_claim(
    store: &MemoryStore,
    stick_id: &StickId,
    kind: ClaimKind,
) -> Result<(), StoreError> {
    store.state().claims.remove(&claim_key(stick_id, kind));
    Ok(())
}
