//! A waited-for link whose sender answered becomes a received stick
//! (ADR 0025), or waits again, saying why.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::download::{mark_received_folder, take_stick};
use ourstick_core::places::check_received_folder;
use ourstick_core::store::JoinedChange;
use ourstick_core::{StickId, StickKind, Ticket};
use tauri::{AppHandle, Manager};

use super::super::{changed, start, work, Receivers};
use super::start_waiting;
use crate::state::AppState;

/// After a change into a received stick failed: the next try.
const AFTER_A_FAILED_CHANGE: Duration = Duration::from_secs(5 * 60);

/// The sender answered: under the `held` mutex, while it is still held and
/// waiting and the app is not quitting, its folder is made and it becomes a
/// received stick (its wait row goes in the same step), then it starts,
/// everything ticked. If that fails, it waits again and says why.
pub(super) async fn change_into_received(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
    added_at_ms: u64,
    name: String,
    folder: PathBuf,
) {
    let receivers = app.state::<Receivers>();
    let state = app.state::<AppState>();
    let changed_into = {
        let held = receivers.held.lock().await;
        if receivers.quitting.load(std::sync::atomic::Ordering::SeqCst)
            || !held.contains_key(&stick_id)
        {
            return;
        }
        let store = state.store.clone();
        let data_dir = state.data_dir.clone();
        let made = {
            let folder = folder.clone();
            work(move || {
                let waits = store
                    .waiting_links()?
                    .iter()
                    .any(|link| link.stick_id == stick_id);
                if !waits {
                    return Ok(false);
                }
                check_received_folder(store.as_ref(), &data_dir, &folder, &stick_id)?;
                std::fs::create_dir_all(&folder)?;
                mark_received_folder(&folder, &stick_id)?;
                Ok(true)
            })
            .await
        };
        match made {
            Ok(false) => return,
            Ok(true) => record(&state, stick_id, &folder, name).await,
            Err(why) => Err(why),
        }
    };
    match changed_into {
        Ok(()) => start(app, stick_id, ticket, folder, false).await,
        Err(why) => {
            tracing::warn!(%why, "a waiting link did not become a received stick");
            // It tries again by itself; said once.
            let message = if why.contains("tries again by itself") {
                format!("It could not start: {why}")
            } else {
                format!("It could not start: {why} OurStick tries again by itself.")
            };
            start_waiting(
                app,
                stick_id,
                ticket,
                added_at_ms,
                AFTER_A_FAILED_CHANGE,
                Some(message),
            )
            .await;
        }
    }
    changed(app);
}

/// Records it as a received stick of the app, with the sender's name; a
/// command-line receive of it is taken over, and its wait ends.
async fn record(
    state: &AppState,
    stick_id: StickId,
    folder: &Path,
    name: String,
) -> Result<(), String> {
    let now = SystemClock.now_ms();
    take_stick(&state.store, stick_id, folder, StickKind::App, now)
        .await
        .map_err(|error| {
            super::super::stopped_words_then(&error, super::super::AfterStop::ByItself)
        })?;
    let store = state.store.clone();
    work(move || {
        // Taken over from the command line: the row was there, so the
        // wait ends here.
        store.remove_waiting_link(&stick_id)?;
        if ourstick_core::manifest::validate_name(&name).is_ok() {
            store.change_joined_stick(&stick_id, &JoinedChange::Name(name))?;
        }
        Ok(())
    })
    .await
}
