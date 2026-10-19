//! "OurStick is still open" (ADR 0028 §6): shown at the first ✕ only, with
//! a picture of this computer's screen and where to click to open the app
//! again. The picture is chosen each time it is shown, nothing cached.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::blocking;
use crate::state::AppState;

/// Which picture the window shows.
#[derive(Debug, Serialize)]
pub struct StillOpenView {
    /// "windows", "macos" or "linux".
    platform: &'static str,
    /// The small icon near the clock is shown (on Linux, only with a
    /// StatusNotifier host: Ubuntu's AppIndicator extension, KDE).
    tray: bool,
}

#[tauri::command]
pub async fn still_open_view() -> Result<StillOpenView, String> {
    let tray = blocking(|| Ok(tray_shown())).await?;
    Ok(StillOpenView {
        platform: std::env::consts::OS,
        tray,
    })
}

/// `OK`: said once, never again. (`Quit OurStick` does not set it.)
#[tauri::command]
pub async fn still_open_ok(state: State<'_, AppState>) -> Result<(), String> {
    let store = state.store.clone();
    blocking(move || {
        let settings = store.settings()?;
        Ok(store.set_settings(&ourstick_core::store::Settings {
            still_open_told: true,
            ..settings
        })?)
    })
    .await
}

/// Whether it was said already. In doubt, yes: never twice by mistake.
pub async fn told(app: &AppHandle) -> bool {
    let store = tauri::Manager::state::<AppState>(app).store.clone();
    blocking(move || Ok(store.settings()?.still_open_told))
        .await
        .unwrap_or(true)
}

/// Whether the tray icon is shown. Linux: when `org.kde.StatusNotifierWatcher`
/// has an owner on the session bus and says a host is registered. Blocking.
#[cfg(target_os = "linux")]
fn tray_shown() -> bool {
    let shown = || -> zbus::Result<bool> {
        let connection = zbus::blocking::Connection::session()?;
        let bus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
        let watcher = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher")?;
        if !bus.name_has_owner(watcher.clone())? {
            return Ok(false);
        }
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            watcher,
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )?;
        proxy.get_property::<bool>("IsStatusNotifierHostRegistered")
    };
    shown().unwrap_or_else(|error| {
        tracing::info!(%error, "no tray host found");
        false
    })
}

#[cfg(not(target_os = "linux"))]
fn tray_shown() -> bool {
    true
}
