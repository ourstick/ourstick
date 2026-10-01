//! The "Stop sharing" dialog, also shown on Quit (docs/gui-spec.md, panel 4):
//! always a confirmation, saying whether someone is still downloading.

use anyhow::Result;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::sharing::{readers_now, refresh_tray, start_sharing_later, stop_serving};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState, Sharing};
use ourstick_core::StickId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Stop sharing one stick.
    StopSharing(StickId),
    /// Quit: every stick stops.
    Quit,
}

/// [`open`] from an event handler of the event loop, where a new window
/// would never load on Windows: from a task instead.
pub fn open_later(app: &AppHandle, mode: Mode) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = open(&app, mode) {
            tracing::warn!(error = %format!("{error:#}"), "cannot show the dialog");
        }
    });
}

/// Shows the dialog for `mode` (one at a time). Not from an event handler
/// (see [`open_later`]) or a synchronous command.
pub fn open(app: &AppHandle, mode: Mode) -> Result<()> {
    if let Some(window) = app.get_webview_window("confirm") {
        let _ = window.close();
    }
    let page = match mode {
        Mode::StopSharing(stick_id) => format!("index.html#stop/{}", stick_hex(&stick_id)),
        Mode::Quit => "index.html#quit".to_owned(),
    };
    let window = WebviewWindowBuilder::new(app, "confirm", WebviewUrl::App(page.into()))
        .title("OurStick")
        .focused(crate::windows::FOCUS_NEW)
        .inner_size(420.0, 230.0)
        .resizable(false)
        .always_on_top(true)
        .center()
        .build()?;
    crate::windows::show_new(&window);
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ConfirmView {
    /// The stick (any stick, for Quit) is being shared now.
    sharing: bool,
    /// How many are downloading right now (from any stick, for Quit).
    readers: usize,
    /// The stick's name (Stop sharing).
    name: Option<String>,
    /// For Quit: the sticks being shared, oldest first, and those someone
    /// downloads from now.
    shared: Vec<String>,
    downloading: Vec<String>,
}

/// What the dialog says: about `stick_id`, or (none) about every stick.
#[tauri::command]
pub async fn confirm_view(
    stick_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConfirmView, String> {
    let all = state
        .sticks_now()
        .await
        .map_err(|error| format!("{error:#}"))?;
    let sharing = state.sticks.lock().await;
    let readers_of = |id| match sharing.get(id) {
        Some(Sharing::On(serving)) => serving.activity.borrow().readers,
        _ => 0,
    };
    let shared_now = |id| sharing.get(id).is_some_and(Sharing::is_on);
    Ok(match stick_id {
        Some(hex) => {
            let id = stick_id_of(&hex)?;
            ConfirmView {
                sharing: shared_now(&id),
                readers: readers_of(&id),
                name: all
                    .iter()
                    .find(|stick| stick.stick_id == id)
                    .map(|stick| stick.name.clone()),
                shared: Vec::new(),
                downloading: Vec::new(),
            }
        }
        None => ConfirmView {
            sharing: sharing.values().any(Sharing::is_on),
            readers: readers_now(&sharing),
            name: None,
            shared: all
                .iter()
                .filter(|stick| shared_now(&stick.stick_id))
                .map(|stick| stick.name.clone())
                .collect(),
            downloading: all
                .iter()
                .filter(|stick| readers_of(&stick.stick_id) > 0)
                .map(|stick| stick.name.clone())
                .collect(),
        },
    })
}

/// Async, as `share::open_share`: a window made from a synchronous command
/// never loads on Windows.
#[tauri::command]
pub async fn open_dialog(
    app: AppHandle,
    quit: bool,
    stick_id: Option<String>,
) -> Result<(), String> {
    let mode = match (quit, stick_id) {
        (true, _) => Mode::Quit,
        (false, Some(hex)) => Mode::StopSharing(stick_id_of(&hex)?),
        (false, None) => return Err("which stick?".to_owned()),
    };
    open(&app, mode).map_err(|error| format!("{error:#}"))
}

/// Stops serving a stick, until the user starts it again (even after a
/// restart). Anyone with its link can no longer download.
#[tauri::command]
pub async fn stop_sharing(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    state
        .set_sharing_wanted(&stick_id, false)
        .await
        .map_err(|error| format!("{error:#}"))?;
    stop_serving(&state, &stick_id).await;
    refresh_tray(&app).await;
    crate::sharing::refresh_active(&app).await;
    let _ = tauri::Emitter::emit(&app, "sharing-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn start_sharing_again(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    state
        .set_sharing_wanted(&stick_id, true)
        .await
        .map_err(|error| format!("{error:#}"))?;
    // A busy stick is shared again, if wanted, when what keeps it busy ends.
    start_sharing_later(&app, stick_id);
    Ok(())
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
