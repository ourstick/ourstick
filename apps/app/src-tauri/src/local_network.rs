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

/// Whether the step is due here: on macOS only, once per device. Blocking.
pub fn step_due(store: &dyn ourstick_core::Store) -> bool {
    needs_step(cfg!(target_os = "macos"), false, || {
        store.local_network_explained().unwrap_or(false)
    })
}

/// On `macos`, unless shown in this run or remembered.
fn needs_step(macos: bool, shown_now: bool, remembered: impl FnOnce() -> bool) -> bool {
    macos && !shown_now && !remembered()
}

/// Waits, before an endpoint is bound, until the step is done (at once
/// when there is none): macOS asks at the first bind, right after it.
pub async fn may_bind(state: &AppState) {
    let mut open = state.network_open.subscribe();
    // The sender lives as long as the state: never closed while waiting.
    let _ = open.wait_for(|open| *open).await;
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

/// "Continue" on the step: every endpoint waiting binds now (macOS asks),
/// and the step is remembered once the device exists.
#[tauri::command]
pub async fn local_network_explained(state: State<'_, AppState>) -> Result<(), String> {
    state.local_network_explained.store(true, Ordering::Relaxed);
    state.network_open.send_replace(true);
    let store = state.store.clone();
    // False before the device is set up: remembered once it is. A failure
    // only means the step comes again next time.
    if let Err(error) = blocking(move || Ok(store.set_local_network_explained()?)).await {
        tracing::warn!(%error, "the Local Network step could not be remembered");
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

    fn state(open: bool) -> AppState {
        use std::sync::Arc;
        AppState {
            data_dir: std::env::temp_dir(),
            store: Arc::new(ourstick_core::MemoryStore::new()),
            sticks: tokio::sync::Mutex::default(),
            disks: Arc::new(ourstick_core::disk::turns::DiskTurns::new(Arc::new(
                ourstick_core::disk::turns::SystemTimer::default(),
            ))),
            local_network_explained: Default::default(),
            keys_wait: Default::default(),
            started_moved: false,
            network_open: tokio::sync::watch::Sender::new(open),
        }
    }

    #[tokio::test]
    async fn endpoints_wait_for_the_step_and_bind_once_it_is_done() {
        let state = std::sync::Arc::new(state(false));
        let waiting = {
            let state = state.clone();
            tokio::spawn(async move { may_bind(&state).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "the step is not done yet");
        state.network_open.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("bound once the step is done")
            .unwrap();
        // With no step due, at once.
        may_bind(&self::state(true)).await;
    }

    #[test]
    fn the_settings_link_is_the_local_network_list() {
        assert!(SETTINGS.starts_with("x-apple.systempreferences:"));
        assert!(SETTINGS.ends_with("Privacy_LocalNetwork"));
    }
}
