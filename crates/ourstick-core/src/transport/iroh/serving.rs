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

/// A store that serves blobs: each version's manifest, and the owner's own
/// store of its files (ADR 0017). Never reused: a store that left keeps its
/// id.
pub(super) type StoreId = u64;

/// Whether a store can send a blob now, looked at before it begins (ADR
/// 0017: its file is there, of its size, with its outboard), on the
/// store's own threads.
#[derive(Clone)]
pub(super) struct Probe {
    pub(super) look: Arc<dyn Fn(Hash) -> bool + Send + Sync>,
    pub(super) runtime: tokio::runtime::Handle,
}

impl Probe {
    /// Looks at `hash`; false if the look could not run.
    pub(super) async fn can_send(&self, hash: Hash) -> bool {
        let look = self.look.clone();
        // On the store's runtime's few blocking threads, kept for minutes,
        // so no thread comes and goes for it, and an answer does not wait
        // behind exports reading a slow disk.
        self.runtime
            .spawn_blocking(move || look(hash))
            .await
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Probe")
    }
}

#[derive(Debug)]
struct Held {
    store: Store,
    /// Blobs of this store that may be served (their hash was checked).
    ready: HashSet<Hash>,
    /// Leaving: no new transfer starts from it.
    leaving: bool,
    /// Transfers in progress from it.
    in_flight: usize,
    probe: Option<Probe>,
}

#[derive(Debug, Default)]
struct State {
    /// Blobs of the current version: its manifest and its files.
    allowed: HashSet<Hash>,
    /// When a reader was last told "not ready".
    last_not_ready: Option<std::time::Instant>,
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
                    probe: held.probe.clone(),
                })
            }
            None => {
                state.last_not_ready = Some(std::time::Instant::now());
                Decision::NotReady
            }
        }
    }

    /// Whether a reader was told "not ready" within the last `window`.
    pub(super) fn not_ready_within(&self, window: std::time::Duration) -> bool {
        self.state()
            .last_not_ready
            .is_some_and(|at| at.elapsed() < window)
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
                probe: None,
            },
        );
    }

    /// `hash` may be served from store `id` (its bytes were checked).
    pub(super) fn set_ready(&self, id: StoreId, hash: Hash) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.ready.insert(hash);
        }
    }

    /// Store `id` is looked at with `probe` before each transfer begins.
    pub(super) fn set_probe(&self, id: StoreId, probe: Probe) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.probe = Some(probe);
        }
    }

    /// Store `id` no longer serves `hash`.
    pub(super) fn remove_ready(&self, id: StoreId, hash: &Hash) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.ready.remove(hash);
        }
    }

    /// Store `id` serves exactly `ready` from now on.
    pub(super) fn replace_ready(&self, id: StoreId, ready: HashSet<Hash>) {
        if let Some(held) = self.state().stores.get_mut(&id) {
            held.ready = ready;
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
    probe: Option<Probe>,
}

impl Guard {
    pub(super) fn store(&self) -> Store {
        self.store.clone()
    }

    /// What to look at before the transfer begins, if its store says.
    pub(super) fn probe(&self) -> Option<Probe> {
        self.probe.clone()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.serving.end(self.id);
    }
}

#[cfg(test)]
mod tests;
