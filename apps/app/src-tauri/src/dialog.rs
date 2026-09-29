//! The "Stop sharing" dialog, also shown on Quit (docs/gui-spec.md, panel 4):
//! always a confirmation, saying whether someone is still downloading.

use anyhow::Result;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::state::{start_sharing, AppState, Sharing};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    StopSharing,
    Quit,
}

/// Shows the dialog for `mode` (one at a time).
pub fn open(app: &AppHandle, mode: Mode) -> Result<()> {
    if let Some(window) = app.get_webview_window("confirm") {
        let _ = window.close();
    }
    let page = match mode {
        Mode::StopSharing => "index.html#stop",
        Mode::Quit => "index.html#quit",
    };
    WebviewWindowBuilder::new(app, "confirm", WebviewUrl::App(page.into()))
        .title("OurStick")
        .focused(crate::windows::FOCUS_NEW)
        .inner_size(420.0, 230.0)
        .resizable(false)
        .always_on_top(true)
        .center()
        .build()?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ConfirmView {
    /// The stick is being shared now.
    sharing: bool,
    /// How many are downloading right now.
    readers: usize,
}

#[tauri::command]
pub async fn confirm_view(state: State<'_, AppState>) -> Result<ConfirmView, String> {
    Ok(match &*state.sharing.lock().await {
        Sharing::On(serving) => ConfirmView {
            sharing: true,
            readers: serving.activity.borrow().readers,
        },
        _ => ConfirmView {
            sharing: false,
            readers: 0,
        },
    })
}

#[tauri::command]
pub fn open_dialog(app: AppHandle, quit: bool) -> Result<(), String> {
    let mode = if quit { Mode::Quit } else { Mode::StopSharing };
    open(&app, mode).map_err(|error| format!("{error:#}"))
}

/// Stops serving the stick. Anyone with the link can no longer download.
#[tauri::command]
pub async fn stop_sharing(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let sharing = std::mem::replace(&mut *state.sharing.lock().await, Sharing::Off);
    if let Sharing::On(serving) = sharing {
        serving.stop().await;
    }
    crate::tray::set_active(&app, false);
    crate::tray::set_sharing(&app, false);
    let _ = tauri::Emitter::emit(&app, "sharing-changed", ());
    Ok(())
}

#[tauri::command]
pub fn start_sharing_again(app: AppHandle) {
    start_sharing(&app);
}

/// Set once the user confirmed Quit: any other way out (the Dock, ⌘Q, the
/// app menu) is held back and shows the dialog first (see main).
pub static QUIT_CONFIRMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Quits the app; sharing stops on the way out (see main).
#[tauri::command]
pub fn quit(app: AppHandle) {
    QUIT_CONFIRMED.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
}
