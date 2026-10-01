//! The app's sticks, and what a stick's life brings (ADR 0009 §5): the
//! list, a new link, removal. Each stops the stick's share first and keeps
//! it busy meanwhile, so nothing starts it halfway.

use ourstick_core::device;
use ourstick_core::locks::{stick_lock, take_retrying};
use ourstick_core::places::check_new_stick_folder;
use ourstick_core::share::mark_drive;
use ourstick_core::stick_life::{give_new_link, remove_stick};
use ourstick_core::stick_names::{folder_name, new_stick_name};
use ourstick_core::{OwnedStick, StickKind};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::sharing::{make_busy, refresh_tray, start_sharing};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState, Sharing};

/// One of the app's sticks, as the list shows it.
#[derive(Debug, Serialize)]
pub struct StickEntry {
    id: String,
    name: String,
    /// "off", "starting", "on", "failed" or "busy".
    sharing: &'static str,
    readers: usize,
}

/// The app's sticks, oldest first.
#[tauri::command]
pub async fn sticks(state: State<'_, AppState>) -> Result<Vec<StickEntry>, String> {
    let sticks = state.app_sticks().map_err(shown)?;
    let sharing = state.sticks.lock().await;
    Ok(sticks
        .into_iter()
        .map(|stick| {
            let (word, readers) = match sharing.get(&stick.stick_id) {
                None | Some(Sharing::Off) => ("off", 0),
                Some(Sharing::Starting) => ("starting", 0),
                Some(Sharing::On(serving)) => ("on", serving.activity.borrow().readers),
                Some(Sharing::Failed(_)) => ("failed", 0),
                Some(Sharing::Busy(_)) => ("busy", 0),
            };
            StickEntry {
                id: stick_hex(&stick.stick_id),
                name: stick.name,
                sharing: word,
                readers,
            }
        })
        .collect())
}

/// "Give it a new link": everyone with the old link loses it; the files
/// stay. Shared again with the new link if it was wanted shared. Returns the
/// stick's new id.
#[tauri::command]
pub async fn new_link(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let old = stick_id_of(&stick_id)?;
    let stick = state.app_stick(&old).map_err(shown)?;
    if !make_busy(&state, &old, "Giving it a new link…").await {
        return Err("the stick is busy; try again in a moment".to_owned());
    }
    let _ = app.emit("sharing-changed", ());
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let locked = stick.clone();
    let made = blocking(move || {
        let secrets = device::open_system_keyring()?;
        // The stick's own lock, as for every change of a stick; not the
        // device node key's, which another older stick may be using.
        let _lock = take_retrying(|| stick_lock(&data_dir, &locked))?;
        Ok(give_new_link(
            store.as_ref(),
            secrets.as_ref(),
            &data_dir,
            &old,
        )?)
    })
    .await;
    state.sticks.lock().await.remove(&old);
    let new = match made {
        Ok(keys) => keys.stick.stick_id,
        Err(error) => {
            if stick.sharing_wanted {
                start_sharing(&app, old, 1).await;
            }
            let _ = app.emit("sharing-changed", ());
            return Err(error);
        }
    };
    if stick.sharing_wanted {
        start_sharing(&app, new, 1).await;
    }
    refresh_tray(&app).await;
    let _ = app.emit("sharing-changed", ());
    let _ = app.emit("sticks-changed", ());
    Ok(stick_hex(&new))
}

/// "Remove this stick": its copied files are deleted (never a linked
/// folder), with its link and keys. Asked for in the window first.
#[tauri::command]
pub async fn remove(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    let stick = state.app_stick(&id).map_err(shown)?;
    if !make_busy(&state, &id, "Removing…").await {
        return Err("the stick is busy; try again in a moment".to_owned());
    }
    let _ = app.emit("sharing-changed", ());
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let removed = blocking(move || {
        let secrets = device::open_system_keyring()?;
        let lock = take_retrying(|| stick_lock(&data_dir, &stick))?;
        let removed = remove_stick(
            store.as_ref(),
            secrets.as_ref(),
            &data_dir,
            &id,
            &crate::drive::AppDrive,
        );
        drop(lock);
        Ok(removed?)
    })
    .await;
    state.sticks.lock().await.remove(&id);
    refresh_tray(&app).await;
    let _ = app.emit("sharing-changed", ());
    let _ = app.emit("sticks-changed", ());
    let _ = app.emit("mount-changed", ());
    removed
}

/// Moves the first stick of an older version into `~/OurStick` if it is
/// `~/OurStick` itself, before a second stick is made there (ADR 0009 §4).
/// Its share stops meanwhile and starts again after, if wanted.
async fn move_first_if_needed(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let home = blocking(crate::drive::sticks_home).await?;
    let first = state
        .app_sticks()
        .map_err(shown)?
        .into_iter()
        .find(|stick| stick.root_path == home);
    let Some(first) = first else {
        return Ok(());
    };
    let id = first.stick_id;
    if !make_busy(state, &id, "Moving into the OurStick folder…").await {
        return Err("the first stick is busy; try again in a moment".to_owned());
    }
    let _ = app.emit("sharing-changed", ());
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let moving = first.clone();
    let moved = blocking(move || {
        let _lock = take_retrying(|| stick_lock(&data_dir, &moving))?;
        let folder = folder_name(&moving.name, |candidate| home.join(candidate).exists());
        let root = crate::drive::move_first(&moving, &folder)?;
        store.move_owned_stick(&moving.stick_id, &root)?;
        let moved = OwnedStick {
            root_path: root,
            ..moving
        };
        crate::drive::mount(&moved)?;
        Ok(())
    })
    .await;
    state.sticks.lock().await.remove(&id);
    if first.sharing_wanted {
        start_sharing(app, id, 1).await;
    }
    let _ = app.emit("mount-changed", ());
    moved
}

/// "New stick…": a stick called `name`, its own folder (or image) shown as
/// a drive, its own keys and link, shared at once. Returns its id.
#[tauri::command]
pub async fn new_stick(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    move_first_if_needed(&app, &state).await?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let made = blocking(move || {
        let secrets = device::open_system_keyring()?;
        // One program makes a stick at a time: names and folders stay
        // unique.
        let _setup = ourstick_core::locks::setup_lock(&data_dir)?;
        let others = crate::state::app_sticks(store.as_ref())?;
        let name = new_stick_name(&name, &others)?;
        let home = crate::drive::sticks_home()?;
        let folder = folder_name(&name, |candidate| home.join(candidate).exists());
        check_new_stick_folder(store.as_ref(), &data_dir, &home.join(&folder))?;
        let (root, image) = crate::drive::create(&folder, &name)?;
        let keys = device::create_stick(
            store.as_ref(),
            secrets.as_ref(),
            &root,
            &name,
            StickKind::App,
        )?;
        let id = keys.stick.stick_id;
        mark_drive(&root, &id)?;
        crate::drive::hide_marker(&root);
        if let Some(image) = image {
            store.set_image_path(&id, &image)?;
        }
        if let Some(letter) = crate::drive::letter_of(&root) {
            store.set_drive_letter(&id, Some(letter))?;
        }
        Ok(id)
    })
    .await?;
    let _ = app.emit("sticks-changed", ());
    let _ = app.emit("mount-changed", ());
    start_sharing(&app, made, 1).await;
    Ok(stick_hex(&made))
}
