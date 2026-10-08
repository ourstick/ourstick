//! ADR 0020 in the app: on macOS, the step that explains the Local Network
//! permission before macOS asks (nothing binds an endpoint before it), and
//! the way to the setting.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::commands::blocking;
use crate::state::AppState;

/// System Settings → Privacy & Security → Local Network.
const SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork";

/// Whether the step is still to be shown: on macOS only, once per device.
pub fn must_explain(state: &AppState) -> bool {
    needs_step(
        cfg!(target_os = "macos"),
        state.local_network_explained.load(Ordering::Relaxed),
        || state.store.local_network_explained().unwrap_or(false),
    )
}

/// On `macos`, unless shown in this run or remembered.
fn needs_step(macos: bool, shown_now: bool, remembered: impl FnOnce() -> bool) -> bool {
    macos && !shown_now && !remembered()
}

/// Remembers the step, once the device exists (it may have been shown
/// before the device was set up, on a first launch).
pub fn remember(state: &AppState) {
    if state.local_network_explained.load(Ordering::Relaxed) {
        if let Err(error) = state.store.set_local_network_explained() {
            tracing::warn!(%error, "the Local Network step could not be remembered");
        }
    }
}

/// "Continue" on the step: remembered, and the sticks that waited for it
/// start (macOS asks then, for the first endpoint bound).
#[tauri::command]
pub async fn local_network_explained(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.local_network_explained.store(true, Ordering::Relaxed);
    let store = state.store.clone();
    // False before the device is set up: remembered once it is.
    blocking(move || Ok(store.set_local_network_explained()?)).await?;
    if state.start_waits.swap(false, Ordering::Relaxed) {
        crate::sharing::start_app(&app);
    }
    Ok(())
}

/// Opens the Local Network list in System Settings.
#[tauri::command]
pub fn open_local_network_settings(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(SETTINGS, None::<&str>)
        .map_err(|error| {
            tracing::warn!(%error, "System Settings could not be opened");
            "System Settings could not be opened: open it, then Privacy & Security, \
             then Local Network."
                .to_owned()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_is_for_macos_once() {
        assert!(needs_step(true, false, || false));
        assert!(!needs_step(true, true, || false), "shown in this run");
        assert!(!needs_step(true, false, || true), "remembered");
        assert!(
            !needs_step(false, false, || false),
            "not on Linux or Windows"
        );
    }

    #[test]
    fn the_settings_link_is_the_local_network_list() {
        assert!(SETTINGS.starts_with("x-apple.systempreferences:"));
        assert!(SETTINGS.ends_with("Privacy_LocalNetwork"));
    }
}
