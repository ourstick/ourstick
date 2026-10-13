//! Links the app waits to receive (ADR 0025), as the SQLite store keeps
//! them: a stick waits here or is received, never both.

use super::{sorted, MemoryStore};
use crate::stick_id::StickId;
use crate::store::{check_time, StoreError, WaitingLink};

pub(super) fn add(store: &MemoryStore, link: &WaitingLink) -> Result<(), StoreError> {
    check_time(link.added_at_ms)?;
    let mut state = store.state();
    let key = link.stick_id.to_bytes();
    if state.joined.contains_key(&key) || state.waiting.contains_key(&key) {
        return Err(StoreError::StickExists);
    }
    state.waiting.insert(key, link.clone());
    Ok(())
}

pub(super) fn list(store: &MemoryStore) -> Vec<WaitingLink> {
    sorted(
        store
            .state()
            .waiting
            .iter()
            .map(|(key, link)| (link.added_at_ms, *key, link.clone())),
    )
}

pub(super) fn remove(store: &MemoryStore, stick_id: &StickId) {
    store.state().waiting.remove(&stick_id.to_bytes());
}
