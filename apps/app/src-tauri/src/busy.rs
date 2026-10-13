//! A stick kept busy while it is renamed, given a new link or removed:
//! nothing starts it meanwhile, and nothing else changes it.

use std::collections::HashMap;

use ourstick_core::StickId;

use crate::state::{AppState, Sharing};

/// Stops serving `stick_id`, if it does, and waits until its endpoint is
/// closed and its locks are released; it is then `Off`. A busy stick is left
/// to what keeps it busy.
pub async fn stop_serving(state: &AppState, stick_id: &StickId) {
    let previous = {
        let mut sticks = state.sticks.lock().await;
        if matches!(sticks.get(stick_id), Some(Sharing::Busy(_))) {
            return;
        }
        sticks.insert(*stick_id, Sharing::Off)
    };
    if let Some(Sharing::On(serving)) = previous {
        serving.stop().await;
    }
}

/// Whether `stick_id` is busy (being renamed, given a new link, removed).
pub async fn is_busy(state: &AppState, stick_id: &StickId) -> bool {
    matches!(
        state.sticks.lock().await.get(stick_id),
        Some(Sharing::Busy(_))
    )
}

/// Marks `stick_id` busy (nothing starts it meanwhile) and stops serving
/// it, so its lock is free. Refused, saying why, if it is busy already or
/// still starting (its share holds the lock and cannot be stopped yet).
pub async fn make_busy(
    state: &AppState,
    stick_id: &StickId,
    doing: &'static str,
) -> Result<(), String> {
    let previous = {
        let mut sticks = state.sticks.lock().await;
        match sticks.get(stick_id) {
            Some(Sharing::Busy(_)) => {
                return Err("the stick is busy; try again in a moment".to_owned())
            }
            Some(Sharing::Starting) => {
                return Err(
                    "the stick is still getting ready to share; try again in a moment".to_owned(),
                )
            }
            _ => {}
        }
        // A copy onto it would go on into a folder being changed (or
        // removed under it): refused while one runs. Read under the sticks'
        // lock: a drop marks its copy first and checks this lock after, so
        // one of the two always sees the other.
        if state
            .copying
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(stick_id)
        {
            return Err(
                "a copy onto the stick is still going on; try again once it is done".to_owned(),
            );
        }
        sticks.insert(*stick_id, Sharing::Busy(doing))
    };
    if let Some(Sharing::On(serving)) = previous {
        serving.stop().await;
    }
    Ok(())
}

/// How many download from any stick now.
pub fn readers_now(sticks: &HashMap<StickId, Sharing>) -> usize {
    sticks
        .values()
        .map(|sharing| match sharing {
            Sharing::On(serving) => serving.activity.borrow().readers,
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn state() -> AppState {
        AppState {
            data_dir: std::env::temp_dir(),
            store: Arc::new(ourstick_core::MemoryStore::new()),
            sticks: tokio::sync::Mutex::new(HashMap::new()),
            disks: Arc::new(ourstick_core::disk::turns::DiskTurns::new(Arc::new(
                ourstick_core::disk::turns::SystemTimer::default(),
            ))),
            copying: std::sync::Mutex::default(),
            local_network_explained: Default::default(),
            keys_wait: Default::default(),
            started_moved: false,
            network_open: tokio::sync::watch::Sender::new(true),
        }
    }

    fn stick(byte: u8) -> StickId {
        ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id()
    }

    fn word(state: &AppState, id: &StickId) -> &'static str {
        match state.sticks.blocking_lock().get(id) {
            None => "none",
            Some(Sharing::Off) => "off",
            Some(Sharing::Starting) => "starting",
            Some(Sharing::On(_)) => "on",
            Some(Sharing::Failed(_)) => "failed",
            Some(Sharing::Busy(_)) => "busy",
        }
    }

    #[test]
    fn a_stick_not_shared_is_made_busy_once() {
        let state = state();
        let id = stick(1);
        tauri::async_runtime::block_on(async {
            assert_eq!(make_busy(&state, &id, "Renaming…").await, Ok(()));
            assert!(is_busy(&state, &id).await);
            let again = make_busy(&state, &id, "Removing…").await.unwrap_err();
            assert!(again.contains("busy"), "{again}");
        });
        assert_eq!(word(&state, &id), "busy");
    }

    #[test]
    fn a_stick_being_copied_onto_is_not_made_busy() {
        let state = state();
        let id = stick(3);
        state.copying.lock().unwrap().insert(id, Arc::default());
        let refused =
            tauri::async_runtime::block_on(make_busy(&state, &id, "Removing…")).unwrap_err();
        assert!(refused.contains("copy"), "{refused}");
        assert!(!tauri::async_runtime::block_on(is_busy(&state, &id)));
        state.copying.lock().unwrap().remove(&id);
        assert_eq!(
            tauri::async_runtime::block_on(make_busy(&state, &id, "Removing…")),
            Ok(())
        );
    }

    #[test]
    fn a_stick_still_starting_is_not_made_busy() {
        let state = state();
        let id = stick(2);
        state.sticks.blocking_lock().insert(id, Sharing::Starting);
        let refused =
            tauri::async_runtime::block_on(make_busy(&state, &id, "Renaming…")).unwrap_err();
        assert!(refused.contains("getting ready"), "{refused}");
        assert_eq!(word(&state, &id), "starting");
    }

    #[test]
    fn stopping_leaves_a_busy_stick_busy_and_turns_others_off() {
        let state = state();
        let (busy, failed) = (stick(3), stick(4));
        state
            .sticks
            .blocking_lock()
            .insert(busy, Sharing::Busy("Removing…"));
        state
            .sticks
            .blocking_lock()
            .insert(failed, Sharing::Failed("no network".to_owned()));
        tauri::async_runtime::block_on(async {
            stop_serving(&state, &busy).await;
            stop_serving(&state, &failed).await;
            assert!(!is_busy(&state, &failed).await);
        });
        assert_eq!(word(&state, &busy), "busy");
        assert_eq!(word(&state, &failed), "off");
    }

    #[test]
    fn nobody_downloads_from_sticks_not_shared() {
        let mut sticks = HashMap::new();
        sticks.insert(stick(5), Sharing::Off);
        sticks.insert(stick(6), Sharing::Busy("Renaming…"));
        assert_eq!(readers_now(&sticks), 0);
        assert_eq!(readers_now(&HashMap::new()), 0);
    }
}
