//! The keeper's state: the current version, the open suitcases, the queue,
//! and where each reader is; what to prepare next and what must leave for it.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::{load_current, Budget, Context, Current, Done, Job, Next, Open, IN_USE, RELOAD_EVERY};
use crate::transport::{BlobHash, Demand};

#[derive(Default)]
pub(super) struct State {
    pub(super) current: Option<Current>,
    /// Bumped with every new version; places of earlier ones are stale.
    pub(super) generation: u64,
    pub(super) open: Vec<Open>,
    pub(super) preparing: Option<(u64, usize)>,
    /// Suitcases to prepare, in turn (numbers in the current version).
    pub(super) queue: VecDeque<usize>,
    /// What each reader (by connection) asks for (ADR 0016, rule 6).
    pub(super) readers: HashMap<u64, Reader>,
    pub(super) last_reload: Option<Instant>,
    /// Places whose last preparation failed (e.g. a linked folder is gone):
    /// not tried again before the time, and the wait grows.
    pub(super) failed: HashMap<Place, (Instant, Duration)>,
}

/// The first wait after a failed preparation…
const FIRST_RETRY: Duration = Duration::from_secs(1);
/// …doubled each time, up to this.
const LONGEST_RETRY: Duration = Duration::from_secs(60);

/// A suitcase's place: (generation of the version, number in it).
pub(super) type Place = (u64, usize);

/// One reader's asks (ADR 0016, rule 6): with several in flight they end in
/// any order, so its place is not simply its latest ask.
#[derive(Debug, Default)]
pub(super) struct Reader {
    /// Places it asked for and was not served yet, when last asked. An ask
    /// waits until its blob is served, or until [`IN_USE`] passes.
    waiting: HashMap<Place, Instant>,
    /// Places it was served from, when last: the highest is where it is
    /// when nothing waits.
    served: HashMap<Place, Instant>,
}

impl Reader {
    /// Forgets what is older than [`IN_USE`].
    fn prune(&mut self, now: Instant) {
        self.waiting
            .retain(|_, asked| now.duration_since(*asked) < IN_USE);
        self.served
            .retain(|_, asked| now.duration_since(*asked) < IN_USE);
    }

    /// Where it is at `now`: its lowest waiting ask, else its highest served
    /// one (never moving back as asks end out of order), of the asks within
    /// [`IN_USE`].
    pub(super) fn place(&self, now: Instant) -> Option<Place> {
        let fresh = |(place, asked): (&Place, &Instant)| {
            (now.duration_since(*asked) < IN_USE).then_some(*place)
        };
        self.waiting
            .iter()
            .filter_map(fresh)
            .min()
            .or_else(|| self.served.iter().filter_map(fresh).max())
    }

    /// A new version: places of earlier ones can never be served again.
    fn keep_only(&mut self, generation: u64) {
        self.waiting.retain(|place, _| place.0 == generation);
        self.served.retain(|place, _| place.0 == generation);
    }
}

impl State {
    pub(super) fn is_underway(&self, number: usize) -> bool {
        let place = (self.generation, number);
        self.preparing == Some(place)
            || self.open.iter().any(|open| open.place == place)
            || self.queue.contains(&number)
    }

    /// A new version: the queue and places refer to the old one (open
    /// suitcases keep serving what they hold); readers' asks in it can never
    /// be served again.
    pub(super) fn next_generation(&mut self) {
        self.generation += 1;
        self.queue.clear();
        let generation = self.generation;
        for reader in self.readers.values_mut() {
            reader.keep_only(generation);
        }
    }

    pub(super) async fn on_demand(&mut self, event: Demand, context: &Context) {
        let (connection, hash, served) = match event {
            Demand::Closed { connection } => {
                self.readers.remove(&connection);
                return;
            }
            Demand::Asked {
                connection,
                hash,
                served,
            } => (connection, hash, served),
        };
        let known = |state: &Self| {
            state
                .current
                .as_ref()
                .is_some_and(|current| current.plan.by_hash.contains_key(&hash))
        };
        if !known(self)
            && self
                .last_reload
                .is_none_or(|at| at.elapsed() >= RELOAD_EVERY)
        {
            self.last_reload = Some(Instant::now());
            match load_current(context).await {
                Ok(Some(current))
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|old| old.manifest_hash != current.manifest_hash) =>
                {
                    self.next_generation();
                    self.current = Some(current);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "the current version could not be read"),
            }
        }
        self.note(connection, hash, served, Instant::now());
    }

    /// Takes in that `connection` asked for `hash` at `now`, once the
    /// version is known.
    pub(super) fn note(&mut self, connection: u64, hash: BlobHash, served: bool, now: Instant) {
        let Some(first) = self
            .current
            .as_ref()
            .and_then(|current| current.plan.by_hash.get(&hash))
            .and_then(|places| places.first().copied())
        else {
            return;
        };
        let generation = self.generation;
        let asked = (generation, first);
        let mut place = asked;
        if served {
            let mut found = None;
            for open in self
                .open
                .iter_mut()
                .filter(|open| open.ready.contains(&hash))
            {
                open.last_used = now;
                if found.is_none() || open.place.0 == generation {
                    found = Some(open.place);
                }
            }
            place = found.unwrap_or(place);
        }
        let reader = self.readers.entry(connection).or_default();
        reader.prune(now);
        if served {
            reader.waiting.remove(&asked);
            reader.waiting.remove(&place);
            reader.served.insert(place, now);
            // Read-ahead: the one after where the reader is, one per reader.
            let Some(at) = reader.place(now) else {
                return;
            };
            let next = at.1 + 1;
            let exists = self
                .current
                .as_ref()
                .is_some_and(|current| next < current.plan.suitcases.len());
            if at.0 == generation && exists && !self.is_underway(next) {
                self.queue.push_back(next);
            }
        } else {
            reader.waiting.insert(asked, now);
            self.wanted_first(first);
        }
    }

    /// Puts `number`, which a reader waits on, at the front of the queue,
    /// behind only the waited-on ones before it in the stick: asks that
    /// straddle a suitcase's end prepare the earlier one first.
    fn wanted_first(&mut self, number: usize) {
        let place = (self.generation, number);
        if self.preparing == Some(place) || self.open.iter().any(|open| open.place == place) {
            return;
        }
        self.queue.retain(|queued| *queued != number);
        let waited = |queued: usize| {
            self.readers
                .values()
                .any(|reader| reader.waiting.contains_key(&(place.0, queued)))
        };
        let at = self
            .queue
            .iter()
            .take_while(|queued| **queued < number && waited(**queued))
            .count();
        self.queue.insert(at, number);
    }

    /// Whether a reader holds the suitcase at `place` (ADR 0016, rule 6): it
    /// waits on it, is on it, or is on the one before, and asked within
    /// [`IN_USE`]. One it was served from and moved past is not held: a
    /// transfer still coming from it holds it itself (ADR 0007).
    pub(super) fn is_held(&self, place: Place, now: Instant) -> bool {
        self.readers.values().any(|reader| {
            let fresh = |asked: &Instant| now.duration_since(*asked) < IN_USE;
            let asking = reader.waiting.values().any(fresh) || reader.served.values().any(fresh);
            asking
                && (reader.waiting.get(&place).is_some_and(fresh)
                    || reader.place(now).is_some_and(|at| {
                        at.0 == place.0 && (at.1 == place.1 || at.1 + 1 == place.1)
                    }))
        })
    }

    /// The next suitcase to prepare, and what must leave for it.
    pub(super) fn next_job(&mut self, budget: &Budget, now: Instant) -> Next {
        let Some(current) = &self.current else {
            return Next::Nothing;
        };
        let generation = self.generation;
        let number = loop {
            let Some(number) = self.queue.pop_front() else {
                return Next::Nothing;
            };
            let place = (generation, number);
            // Failed lately: dropped from the queue; the reader asks again.
            if self.failed.get(&place).is_some_and(|(at, _)| now < *at) {
                continue;
            }
            if number < current.plan.suitcases.len()
                && self.preparing != Some(place)
                && !self.open.iter().any(|open| open.place == place)
            {
                break number;
            }
        };
        let planned = &current.plan.suitcases[number];
        let need = planned.bytes;
        // Room first: the ones used longest ago leave, never one a reader
        // holds.
        let mut evictable: Vec<usize> = (0..self.open.len())
            .filter(|index| !self.is_held(self.open[*index].place, now))
            .collect();
        evictable.sort_by_key(|index| self.open[*index].last_used);
        let mut free = budget.available();
        let mut leaving = Vec::new();
        for index in evictable.iter().copied() {
            if free >= need {
                break;
            }
            free += self.open[index].bytes;
            leaving.push(index);
        }
        let beyond = need > budget.total();
        let fits = if beyond {
            // A file bigger than the budget: alone, once all else can go.
            leaving.len() == evictable.len() && evictable.len() == self.open.len()
        } else {
            free >= need
        };
        if !fits {
            self.queue.push_front(number);
            return Next::NoRoom;
        }
        leaving.sort_unstable_by(|a, b| b.cmp(a));
        let evict = leaving
            .into_iter()
            .map(|index| {
                let open = self.open.swap_remove(index);
                (open.id, open.bytes)
            })
            .collect();
        let files = current.jobs(planned.entries.clone());
        self.preparing = Some((generation, number));
        Next::Job(Job {
            place: (generation, number),
            files,
            bytes: need,
            evict,
            beyond,
        })
    }

    /// Takes in a finished job; true if it found no room (try again soon).
    pub(super) fn done(&mut self, done: Done) -> bool {
        if self.preparing == Some(done.place) {
            self.preparing = None;
        }
        match done.result {
            Ok(Some(prepared)) => {
                self.failed.remove(&done.place);
                if !prepared.changed.is_empty() {
                    tracing::info!(
                        files = prepared.changed.len(),
                        "files changed since they were read; the next pass reads them again"
                    );
                }
                self.open.push(Open {
                    id: prepared.id,
                    place: done.place,
                    bytes: done.bytes,
                    last_used: Instant::now(),
                    ready: prepared.ready,
                });
                false
            }
            Ok(None) => {
                if done.place.0 == self.generation {
                    self.queue.push_front(done.place.1);
                }
                true
            }
            Err(error) => {
                let wait = self
                    .failed
                    .get(&done.place)
                    .map_or(FIRST_RETRY, |(_, wait)| (*wait * 2).min(LONGEST_RETRY));
                tracing::warn!(%error, wait_s = wait.as_secs(), "a suitcase could not be prepared");
                self.failed
                    .insert(done.place, (Instant::now() + wait, wait));
                false
            }
        }
    }
}
