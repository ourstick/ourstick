//! "Move to another folder…": what came moves with the folder (core
//! `received::move_received_folder`); receiving stops meanwhile and goes on
//! from the new place, unless it was paused.

use std::path::{Path, PathBuf};

use ourstick_core::locks::{download_lock, take_retrying, Lock};
use ourstick_core::received::move_received_folder;
use tauri::{AppHandle, Emitter, State};

use super::{read_ticket, start, Receivers};
use crate::state::{stick_id as stick_id_of, AppState};

/// Moves the received stick `stick_id`'s folder into `into`.
#[tauri::command]
pub async fn move_received(
    stick_id: String,
    into: String,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    // The stick's folder goes inside `into`: as one received there would.
    let inside = Path::new(&into).join("_");
    if let Some(why) = super::into::refused(&inside, super::into::home().as_deref()) {
        return Err(why.to_owned());
    }
    if !receivers.is_held(&id).await {
        return Err("This is not received here any more.".to_owned());
    }
    let store = state.store.clone();
    let paused =
        super::work(move || Ok(store.joined_stick(&id)?.is_some_and(|joined| joined.paused)))
            .await?;
    let held = receivers.release(&id).await;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let moved = super::work(move || -> anyhow::Result<(Lock, Result<PathBuf, String>)> {
        let lock = match held {
            Some(lock) => lock,
            None => take_retrying(|| download_lock(&data_dir, &id))?,
        };
        let moved = move_received_folder(store.as_ref(), &data_dir, &id, Path::new(&into), &lock)
            .map_err(|error| error.to_string());
        Ok((lock, moved))
    })
    .await;
    let (lock, moved) = match moved {
        Ok(done) => done,
        Err(why) => {
            receivers.hold_again(&app, id, None, why.clone()).await;
            let _ = app.emit("received-changed", ());
            return Err(why);
        }
    };
    receivers.hold(id, Some(lock)).await;
    // From where it is now (or where it stayed): as before, unless paused.
    if !paused {
        let store = state.store.clone();
        let again = super::work(move || {
            let joined = store
                .joined_stick(&id)?
                .ok_or_else(|| anyhow::anyhow!("this is not received here any more"))?;
            let secrets = crate::keys::secrets(store.as_ref())?;
            Ok((read_ticket(secrets.as_ref(), &id)?, joined.output_dir))
        })
        .await;
        match again {
            Ok((ticket, folder)) => start(&app, id, ticket, folder, false).await,
            Err(why) => receivers.hold_again(&app, id, None, why).await,
        }
    } else {
        receivers.show(id, |shown| shown.state = "paused");
    }
    let _ = app.emit("received-changed", ());
    moved.map(|_| ())
}
