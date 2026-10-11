//! Sharing the app's sticks (ADR 0009): each its own endpoint, locks,
//! files keeper and watcher, started one after another; what stopped halfway
//! (a removal, a new link) finished first, at start and on every mount.

use std::collections::HashMap;

use anyhow::Result;
use ourstick_core::secrets::check_secret_store;
use ourstick_core::stick_life::{finish_new_links, finish_removals};
use ourstick_core::{device, locks, StickId};
use tauri::{AppHandle, Emitter, Manager};

pub use crate::busy::{is_busy, make_busy, readers_now, stop_serving};
use crate::serving;
use crate::state::{AppState, Sharing};

/// Tells the window about a problem of the whole app.
fn problem(app: &AppHandle, message: String) {
    crate::problems::failed(app, None, message);
}

/// Tells the window about several problems of the whole app, as one.
fn problems(app: &AppHandle, messages: Vec<String>) {
    if !messages.is_empty() {
        problem(app, messages.join("\n"));
    }
}

/// What stopped halfway (a removal, a new link), finished; the failures,
/// told. At start and after a drive is mounted (a new link waits for its
/// drive). Blocking.
fn finish_pending(state: &AppState) -> Result<Vec<String>> {
    let secrets = crate::keys::secrets(state.store.as_ref())?;
    let store = state.store.as_ref();
    let mut failed = Vec::new();
    for (name, error) in finish_removals(
        store,
        secrets.as_ref(),
        &state.data_dir,
        &crate::drive::AppDrive,
    )? {
        failed.push(format!(
            "Removing \u{201c}{name}\u{201d} is not finished: {}",
            crate::words::told(&anyhow::Error::from(error))
        ));
    }
    for (name, error) in finish_new_links(store, secrets.as_ref(), &state.data_dir)? {
        failed.push(format!(
            "Changing the sharing link of \u{201c}{name}\u{201d} is not finished: {}",
            crate::words::told(&anyhow::Error::from(error))
        ));
    }
    for (name, error) in
        ourstick_core::received::finish_forgetting(store, secrets.as_ref(), &state.data_dir)?
    {
        failed.push(format!(
            "Forgetting \u{201c}{name}\u{201d} is not finished: {}",
            crate::words::told(&anyhow::Error::from(error))
        ));
    }
    Ok(failed)
}

/// [`finish_pending`] off the async runtime; its failures told.
pub(crate) async fn finish_pending_off_thread(app: &AppHandle) {
    // Not before the keys' step (ADR 0021): start_app does it after.
    if crate::keys::waiting(&app.state::<AppState>()) {
        return;
    }
    let handle = app.clone();
    let finished =
        tauri::async_runtime::spawn_blocking(move || finish_pending(&handle.state::<AppState>()))
            .await;
    match finished {
        Ok(Ok(failed)) => problems(app, failed),
        Ok(Err(error)) => problem(app, crate::words::told(&error)),
        Err(error) => tracing::warn!(%error, "finishing what waited did not finish"),
    }
}

/// At start, each stick plugged in: what a copy cut short by a Quit left
/// (hidden, never shared) is removed, so it takes no room. A stick not
/// there now, or dropped onto already, is cleared at its next drop.
async fn clear_cut_copies(app: &AppHandle) {
    let handle = app.clone();
    let cleared = tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        let state = handle.state::<AppState>();
        for stick in state.app_sticks()? {
            crate::stick_drop::clear_cut_copy(&state, &stick);
        }
        Ok(())
    })
    .await;
    if let Ok(Err(error)) = cleared {
        tracing::warn!(error = %crate::words::told(&error), "copies cut short not cleared");
    }
}

/// At start: this device recorded once (never by each share: they start
/// one after another and would refuse each other's setup lock), the macOS
/// image of an older stick recorded, what stopped halfway finished; then
/// every stick plugged in, and those the user wants shared, shared.
pub fn start_app(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>> {
            let state = handle.state::<AppState>();
            // What a preview left behind (it keeps nothing).
            let _ = std::fs::remove_dir_all(state.data_dir.join("previews"));
            let sticks = state.app_sticks()?;
            // Windows: File Explorer entries of sticks no longer here.
            let ids: Vec<StickId> = sticks.iter().map(|stick| stick.stick_id).collect();
            crate::drive::clean_up_left_behind(&ids);
            // macOS: the first stick's image, from before images were
            // recorded, goes to the oldest stick only, if it is there and no
            // other stick has it; first, as it needs no keys (a keychain
            // that fails must not leave the stick without its image).
            if let (Some(oldest), Some(image)) = (sticks.first(), crate::drive::first_image()) {
                let unclaimed = !sticks
                    .iter()
                    .any(|stick| stick.image_path.as_deref() == Some(image.as_path()));
                if oldest.image_path.is_none() && unclaimed && image.exists() {
                    state.store.set_image_path(&oldest.stick_id, &image)?;
                }
            }
            let secrets = crate::keys::secrets(state.store.as_ref())?;
            check_secret_store(secrets.as_ref())?;
            let backend = crate::keys::backend(state.store.as_ref())?;
            {
                let _setup = locks::setup_lock(&state.data_dir)?;
                device::open_device(
                    state.store.as_ref(),
                    &state.data_dir,
                    secrets.as_ref(),
                    backend,
                )?;
            }
            crate::local_network::remember(&state);
            crate::keys::finish_move(&state);
            for stick in sticks {
                // A move of the first stick stopped halfway is finished.
                let home = crate::drive::sticks_home()?;
                let folder = ourstick_core::stick_names::folder_name(&stick.name, |candidate| {
                    home.join(candidate).exists()
                });
                if let Some(root) = crate::drive::find_moved(&stick, &folder)? {
                    state.store.move_owned_stick(&stick.stick_id, &root)?;
                }
            }
            finish_pending(&state)
        })
        .await;
        match prepared {
            Ok(Ok(failed)) => problems(&app, failed),
            Ok(Err(error)) => problem(
                &app,
                format!("OurStick could not start: {}", crate::words::told(&error)),
            ),
            Err(error) => tracing::warn!(%error, "the start did not finish"),
        }
        plug_in_all(&app).await;
        clear_cut_copies(&app).await;
        // The sticks it receives, kept up to date again.
        crate::receiving::start_all(&app).await;
    });
}

/// Plugs `stick` in (mounts its image, or puts it in the sidebar) and
/// records where its folder is now; whether a running share must start
/// again to follow it (always after an image is attached: its watch is
/// gone; elsewhere only if the folder moved). Blocking.
fn plug_in(state: &AppState, stick_id: &StickId) -> Result<bool> {
    let stick = state.app_stick(stick_id)?;
    let root = crate::drive::mount(&stick)?;
    let moved = root != stick.root_path;
    if moved {
        state.store.move_owned_stick(stick_id, &root)?;
    }
    // Windows: keep the letter it got, to ask for it after a restart.
    let letter = crate::drive::letter_of(&root);
    if letter.is_some() && letter != stick.drive_letter {
        state.store.set_drive_letter(stick_id, letter)?;
    }
    Ok(moved || cfg!(target_os = "macos"))
}

/// [`plug_in`] off the async runtime; a failure is told (sharing still
/// serves the last version).
async fn plug_in_off_thread(app: &AppHandle, stick_id: StickId) -> bool {
    let handle = app.clone();
    let plugged = tauri::async_runtime::spawn_blocking(move || {
        plug_in(&handle.state::<AppState>(), &stick_id)
    })
    .await;
    match plugged {
        Ok(Ok(follow)) => follow,
        Ok(Err(error)) => {
            tracing::warn!(error = %format!("{error:#}"), "a stick could not be mounted");
            crate::problems::failed(
                app,
                Some(stick_id),
                format!(
                    "The stick could not be mounted: {}",
                    crate::words::told(&error)
                ),
            );
            true
        }
        Err(error) => {
            tracing::warn!(%error, "mounting did not finish");
            true
        }
    }
}

/// Every stick plugged in, then shared if the user wants it shared, one
/// after another.
async fn plug_in_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let sticks = match state.sticks_now().await {
        Ok(sticks) => sticks,
        Err(error) => return problem(app, crate::words::told(&error)),
    };
    for stick in &sticks {
        plug_in_off_thread(app, stick.stick_id).await;
    }
    let _ = app.emit("mount-changed", ());
    refresh_tray(app).await;
    // Drives just plugged in (macOS: images attached only now) let what
    // waited for them finish.
    finish_pending_off_thread(app).await;
    let wanted: Vec<StickId> = sticks
        .iter()
        .filter(|stick| stick.sharing_wanted)
        .map(|stick| stick.stick_id)
        .collect();
    for id in &wanted {
        start_sharing(app, *id).await;
    }
}

/// Mount (from the window or the tray): plugs the stick in, finishes what
/// waited for its drive, and shares it again from where it is now if the
/// user wants it shared.
pub fn mount_and_share(app: &AppHandle, stick_id: StickId) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if is_busy(&app.state::<AppState>(), &stick_id).await {
            crate::problems::failed(
                &app,
                Some(stick_id),
                "The stick is busy; mount it again in a moment.".to_owned(),
            );
            return;
        }
        let restart = plug_in_off_thread(&app, stick_id).await;
        finish_pending_off_thread(&app).await;
        let _ = app.emit("mount-changed", ());
        refresh_tray(&app).await;
        let state = app.state::<AppState>();
        let wanted = state
            .stick_now(&stick_id)
            .await
            .is_ok_and(|stick| stick.sharing_wanted);
        if !wanted {
            return;
        }
        {
            let sticks = state.sticks.lock().await;
            let running = sticks.get(&stick_id).is_some_and(Sharing::is_on);
            if running && !restart {
                return;
            }
        }
        // The watcher follows the folder where it is now.
        stop_serving(&state, &stick_id).await;
        start_sharing(&app, stick_id).await;
    });
}

/// Starts sharing `stick_id` and waits until it runs or has failed. The UI
/// hears `sharing-changed` when it starts and when it is done. Nothing
/// happens while the stick is busy or already shared.
pub async fn start_sharing(app: &AppHandle, stick_id: StickId) {
    let state = app.state::<AppState>();
    {
        let mut sticks = state.sticks.lock().await;
        if sticks
            .get(&stick_id)
            .is_some_and(|sharing| sharing.is_on() || matches!(sharing, Sharing::Busy(_)))
        {
            return;
        }
        sticks.insert(stick_id, Sharing::Starting);
    }
    let _ = app.emit("sharing-changed", ());
    refresh_tray(app).await;
    let started = serving::start(
        app.clone(),
        state.data_dir.clone(),
        state.store.clone(),
        stick_id,
    )
    .await;
    let mut sticks = state.sticks.lock().await;
    // Busy meanwhile (removed, renamed): this share is not wanted.
    if !matches!(sticks.get(&stick_id), Some(Sharing::Starting)) {
        drop(sticks);
        if let Ok(serving) = started {
            serving.stop().await;
        }
        return;
    }
    sticks.insert(
        stick_id,
        match started {
            Ok(serving) => Sharing::On(serving),
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "sharing did not start");
                Sharing::Failed(crate::words::told(&error))
            }
        },
    );
    drop(sticks);
    refresh_tray(app).await;
    let _ = app.emit("sharing-changed", ());
}

/// [`start_sharing`] in the background.
pub fn start_sharing_later(app: &AppHandle, stick_id: StickId) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { start_sharing(&app, stick_id).await });
}

/// Stops every share (Quit).
pub async fn stop_all(state: &AppState) {
    let sticks: HashMap<StickId, Sharing> = std::mem::take(&mut *state.sticks.lock().await);
    for sharing in sticks.into_values() {
        if let Sharing::On(serving) = sharing {
            serving.stop().await;
        }
    }
}

pub use crate::refresh::{refresh_active, refresh_tray, watch_mount};
