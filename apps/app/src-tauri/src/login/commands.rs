//! The window's side of start at login (ADR 0024): the check in About,
//! and the notice said once.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::blocking;
use crate::state::AppState;

#[derive(Serialize)]
pub struct StartAtLoginView {
    /// What the check shows: the choice, off if the system turned it off.
    on: bool,
    /// The window said once that the app starts at login.
    told: bool,
}

#[tauri::command]
pub async fn start_at_login(state: State<'_, AppState>) -> Result<StartAtLoginView, String> {
    let store = state.store.clone();
    blocking(move || {
        Ok(StartAtLoginView {
            on: super::shown(store.as_ref()),
            told: store.settings()?.start_at_login_told,
        })
    })
    .await
}

/// The user's choice, from About; the tray's check follows.
#[tauri::command]
pub async fn set_start_at_login(
    on: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let store = state.store.clone();
    blocking(move || super::set_by_user(store.as_ref(), on)).await?;
    changed(&app).await;
    Ok(())
}

/// The notice was read: not said again.
#[tauri::command]
pub async fn start_at_login_told(state: State<'_, AppState>) -> Result<(), String> {
    let store = state.store.clone();
    blocking(move || {
        let settings = store.settings()?;
        Ok(store.set_settings(&ourstick_core::store::Settings {
            start_at_login_told: true,
            ..settings
        })?)
    })
    .await
}

/// The tray and every window show the choice anew.
pub async fn changed(app: &AppHandle) {
    crate::sharing::refresh_tray(app).await;
    let _ = app.emit("start-at-login-changed", ());
}
