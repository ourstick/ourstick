//! What the owner serves, decided in one place (ADR 0005, ADR 0007): a plain
//! get of a raw blob that the current version allows and that a store holds
//! ready. The decision and the registration of the transfer happen under one
//! lock, so a store cannot start to leave between them: a store leaves only
//! once no transfer holds a [`Guard`] on it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use iroh_blobs::api::Store;
use iroh_blobs::Hash;
use tokio::sync::Notify;

/// A store that serves blobs: today the one cache; with ADR 0007, each
/// suitcase. Never reused: a store that left keeps its id.
pub(super) type StoreId = u64;

#[derive(Debug)]
struct Held {
    store: Store,
    /// Blobs of this store that may be served (their hash was checked).
    ready: HashSet<Hash>,
    /// Leaving: no new transfer starts from it.
    leaving: bool,
    /// Transfers in progress from it.
    in_flight: usize,
}

#[derive(Debug, Default)]
struct State {
    /// Blobs of the current version: its manifest and its files.
    allowed: HashSet<Hash>,
    stores: HashMap<StoreId, Held>,
}

/// The answer to a request for a blob.
#[derive(Debug)]
pub(super) enum Decision {
    /// Served from the guarded store.
    Serve(Guard),
    /// Allowed, but no store holds it ready (yet): "try again later"
    /// (`ERR_LIMIT`), which the reader waits on.
    NotReady,
    /// Not a plain blob of the current version: refused (`ERR_PERMISSION`).
    Refuse,
}

/// The shared state; cheap to clone.
#[derive(Debug, Clone, Default)]
pub(super) struct Serving {
    state: Arc<Mutex<State>>,
    /// Woken whenever a transfer ends.
    ended: Arc<Notify>,
}

impl Serving {
    fn state(&self) -> MutexGuard<'_, State> {
        // Every update is a few set operations; a panic cannot leave them
        // half done in a way that serves what it must not.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The single decision: what a peer asking for `hash` gets.
    pub(super) fn decide(&self, hash: Hash, is_blob: bool) -> Decision {
        let mut state = self.state();
        if !is_blob || !state.allowed.contains(&hash) {
            return Decision::Refuse;
        }
        let ready = state
            .stores
            .iter_mut()
            .find(|(_, held)| !held.leaving && held.ready.contains(&hash));
        match ready {
            Some((&id, held)) => {
                held.in_flight += 1;
                Decision::Serve(Guard {
                    serving: self.clone(),
                    id,
                    store: held.store.clone(),
                })
            }
            None => Decision::NotReady,
        }
    }

    /// From now on, only these blobs are served.
    pub(super) fn allow(&self, allowed: HashSet<Hash>) {
        self.state().allowed = allowed;
    }

    /// Adds a store with nothing ready yet.
    pub(super) fn add_store(&self, id: StoreId, store: Store) {
        self.state().stores.insert(
            id,
            Held {
                store,
                ready: HashSet::new(),
                leaving: false,
                in_flight: 0,
            },
        );
    }

    /// `hash` may be served from store `id` (its bytes were checked).
    pub(super) fn set_ready(&self, id: StoreId, hash: Hash) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.ready.insert(hash);
        }
    }

    /// Keeps as ready in store `id` only what `keep` says.
    pub(super) fn retain_ready(&self, id: StoreId, keep: impl Fn(&Hash) -> bool) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.ready.retain(|hash| keep(hash));
        }
    }

    /// Whether store `id` holds `hash` ready.
    #[cfg(test)]
    pub(super) fn is_ready(&self, id: StoreId, hash: &Hash) -> bool {
        self.state()
            .stores
            .get(&id)
            .is_some_and(|held| held.ready.contains(hash))
    }

    /// Takes store `id` out of service: no new transfer starts from it, and
    /// once none is in progress it is handed back to be closed. `None` if
    /// there is no such store.
    pub(super) async fn remove_store(&self, id: StoreId) -> Option<Store> {
        {
            let mut state = self.state();
            let held = state.stores.get_mut(&id)?;
            held.leaving = true;
        }
        loop {
            // Registered before the check, so an end between the two is not
            // missed.
            let ended = self.ended.notified();
            {
                let mut state = self.state();
                if state
                    .stores
                    .get(&id)
                    .is_some_and(|held| held.in_flight == 0)
                {
                    return state.stores.remove(&id).map(|held| held.store);
                }
            }
            ended.await;
        }
    }

    fn end(&self, id: StoreId) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.in_flight = held.in_flight.saturating_sub(1);
        }
        self.ended.notify_waiters();
    }
}

/// A transfer in progress from a store; the store stays until it drops.
#[derive(Debug)]
pub(super) struct Guard {
    serving: Serving,
    id: StoreId,
    store: Store,
}

impl Guard {
    pub(super) fn store(&self) -> Store {
        self.store.clone()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.serving.end(self.id);
    }
}

#[cfg(test)]
mod tests;
