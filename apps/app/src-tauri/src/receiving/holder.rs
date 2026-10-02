//! The received sticks the app holds: each with its download lock and its
//! running receive, changed under one mutex start to end.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ourstick_core::download::Cancel;
use ourstick_core::locks::Lock;
use ourstick_core::StickId;
use tauri::{AppHandle, Manager};

use super::{lock, stopped, Shown};
use crate::state::AppState;

/// One received stick the app holds.
pub(super) struct Held {
    /// Its download lock, once this program has it (another program may
    /// hold it for a while: then it is taken again at the next start).
    pub(super) lock: Option<Lock>,
    /// Its receive, while one runs.
    pub(super) running: Option<(Arc<Cancel>, tauri::async_runtime::JoinHandle<()>)>,
}

impl Held {
    /// Stops its receive, if one runs, and waits until it ended.
    pub(super) async fn stop(&mut self) {
        if let Some((cancel, task)) = self.running.take() {
            cancel.cancel();
            let _ = task.await;
        }
    }
}

/// The app's received sticks, by id. Every change of one (start, pause,
/// resume, forget, Quit) is made under the one `held` mutex, start to end:
/// two of them never overlap on a stick.
#[derive(Default)]
pub struct Receivers {
    pub(super) held: tokio::sync::Mutex<HashMap<StickId, Held>>,
    shown: Mutex<HashMap<StickId, Shown>>,
    /// What each stick's latest round saved under another name (ADR 0010):
    /// from its plan, so only names in the stick now; none before a round
    /// has planned (the window then reads what is recorded).
    saved_as: Mutex<HashMap<StickId, Vec<ourstick_core::download::SavedAs>>>,
    /// What each stick's latest round found at its top level (ADR 0012),
    /// for choosing what to get with the sender away.
    contents: Mutex<HashMap<StickId, Vec<ourstick_core::download::TopEntry>>>,
    /// Quit began: nothing starts any more.
    pub(super) quitting: std::sync::atomic::AtomicBool,
}

impl Receivers {
    pub fn shown(&self, stick_id: &StickId) -> Shown {
        lock(&self.shown).get(stick_id).cloned().unwrap_or_default()
    }

    pub(super) fn show(&self, stick_id: StickId, change: impl FnOnce(&mut Shown)) {
        change(lock(&self.shown).entry(stick_id).or_default());
    }

    /// What each stick's latest round saved under another name, for the
    /// sticks a round has planned for since the app started (the tests'
    /// view; the window takes [`Receivers::saved_as_lists`]).
    #[cfg(test)]
    pub fn saved_as(&self) -> HashMap<StickId, Vec<ourstick_core::download::SavedAs>> {
        lock(&self.saved_as).clone()
    }

    /// The same, as the window gets it (at most its first lines each): no
    /// copy of a long list on every refresh.
    pub fn saved_as_lists(&self) -> HashMap<StickId, super::words::SavedAsList> {
        lock(&self.saved_as)
            .iter()
            .map(|(stick_id, lines)| (*stick_id, super::words::saved_as_views(lines)))
            .collect()
    }

    /// What `stick_id`'s latest round found at its top level, if a round
    /// has read its manifest since the app started.
    pub fn contents(&self, stick_id: &StickId) -> Option<Vec<ourstick_core::download::TopEntry>> {
        lock(&self.contents).get(stick_id).cloned()
    }

    /// A round read the stick's manifest: what is at its top level now.
    pub(super) fn contents_now(
        &self,
        stick_id: StickId,
        entries: &[ourstick_core::download::TopEntry],
    ) {
        lock(&self.contents).insert(stick_id, entries.to_vec());
    }

    /// A round planned: what it saves under another name starts empty.
    pub(super) fn round_planned(&self, stick_id: StickId) {
        lock(&self.saved_as).insert(stick_id, Vec::new());
    }

    /// One more name the round saves under another.
    pub(super) fn saved_as_too(&self, stick_id: StickId, line: ourstick_core::download::SavedAs) {
        lock(&self.saved_as).entry(stick_id).or_default().push(line);
    }

    /// The received sticks coming in now, for the Quit dialog.
    pub fn downloading(&self) -> Vec<StickId> {
        lock(&self.shown)
            .iter()
            .filter(|(_, shown)| matches!(shown.state, "downloading" | "getting_more"))
            .map(|(stick_id, _)| *stick_id)
            .collect()
    }

    /// Holds `stick_id`, with its lock if it was taken (not running yet).
    pub async fn hold(&self, stick_id: StickId, lock: Option<Lock>) {
        self.held.lock().await.insert(
            stick_id,
            Held {
                lock,
                running: None,
            },
        );
    }

    pub async fn is_held(&self, stick_id: &StickId) -> bool {
        self.held.lock().await.contains_key(stick_id)
    }

    /// "Pause": stops `stick_id`'s receive and records the pause, under the
    /// one mutex (a Resume never lands between them).
    pub async fn pause(&self, app: &AppHandle, stick_id: &StickId) -> Result<(), String> {
        let mut held = self.held.lock().await;
        let Some(entry) = held.get_mut(stick_id) else {
            return Err("This stick is not received here any more.".to_owned());
        };
        entry.stop().await;
        let store = app.state::<AppState>().store.clone();
        let id = *stick_id;
        let recorded =
            super::work(move || {
                Ok(store
                    .change_joined_stick(&id, &ourstick_core::store::JoinedChange::Paused(true))?)
            })
            .await;
        if let Err(why) = recorded {
            // Stopped all the same: said, rather than shown as running.
            stopped(app, *stick_id, why.clone());
            return Err(why);
        }
        self.show(*stick_id, |shown| {
            *shown = Shown {
                state: "paused",
                ..Shown::default()
            }
        });
        Ok(())
    }

    /// Holds `stick_id` again after a forget that failed, saying why.
    pub async fn hold_again(
        &self,
        app: &AppHandle,
        stick_id: StickId,
        lock: Option<Lock>,
        why: String,
    ) {
        self.hold(stick_id, lock).await;
        stopped(app, stick_id, why);
    }

    /// Stops and lets go of `stick_id` (forget); its lock, if this program
    /// had it.
    pub async fn release(&self, stick_id: &StickId) -> Option<Lock> {
        let mut held = self.held.lock().await;
        // What the window showed of it goes, held or not.
        lock(&self.saved_as).remove(stick_id);
        lock(&self.contents).remove(stick_id);
        let mut entry = held.remove(stick_id)?;
        entry.stop().await;
        lock(&self.shown).remove(stick_id);
        entry.lock
    }

    /// Stops every receive (Quit): each keeps what it received, and goes on
    /// where it was next time. At most a few seconds.
    pub async fn stop_all(&self) {
        self.quitting
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut held = self.held.lock().await;
        let running: Vec<_> = held
            .values_mut()
            .filter_map(|held| held.running.take())
            .collect();
        for (cancel, _) in &running {
            cancel.cancel();
        }
        for (_, task) in running {
            // A round may take its grace (10 s) to finish the file it places.
            let _ = tokio::time::timeout(Duration::from_secs(12), task).await;
        }
    }
}
