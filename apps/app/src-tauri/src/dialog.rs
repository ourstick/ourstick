//! The "Stop sharing" dialog, also shown on Quit and on the main window's ✕
//! (docs/gui-spec.md, panel 4): always a confirmation, saying whether someone
//! is still downloading.

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
    /// The main window's ✕: Quit, or minimize the window instead.
    Close,
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
    // A label of its own each time: the one closed now may not be gone yet,
    // and a new window under its label would not be made.
    static OPENED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for (label, window) in app.webview_windows() {
        if label.starts_with("confirm") {
            let _ = window.close();
        }
    }
    let label = format!(
        "confirm-{}",
        OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let page = match mode {
        Mode::StopSharing(stick_id) => format!("index.html#stop/{}", stick_hex(&stick_id)),
        Mode::Quit => "index.html#quit".to_owned(),
        Mode::Close => "index.html#close".to_owned(),
    };
    // Quit also lists the sticks coming in: a paragraph more.
    let receiving = matches!(mode, Mode::Quit | Mode::Close)
        && !app
            .state::<crate::receiving::Receivers>()
            .downloading()
            .is_empty();
    // Quit also has the start at login check: two lines more.
    let quitting = matches!(mode, Mode::Quit | Mode::Close);
    let height = match (quitting, receiving) {
        (true, true) => 370.0,
        (true, false) => 280.0,
        (false, _) => 230.0,
    };
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(page.into()))
        .title("OurStick")
        .focused(crate::windows::FOCUS_NEW)
        .inner_size(420.0, height)
        .resizable(false)
        .always_on_top(true)
        .center()
        .build()?;
    crate::windows::show_new(&window);
    Ok(())
}

/// The main window's ✕, on every system: the tray is a help, never the only
/// way to quit (many never see it). With something going on, the Quit
/// dialog says what stops, with Minimize first; with nothing, the app quits.
pub fn close_requested(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if going_on(&app).await {
            if let Err(error) = open(&app, Mode::Close) {
                tracing::warn!(error = %format!("{error:#}"), "cannot show the dialog");
            }
        } else {
            quit(app).await;
        }
    });
}

/// Whether quitting would stop something: a stick being shared, or a
/// stick received and kept up to date (or waiting for its sender, ADR
/// 0025).
async fn going_on(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    state.sticks.lock().await.values().any(Sharing::is_on)
        || app.state::<crate::receiving::Receivers>().following()
}

/// Minimize, the Close dialog's other choice: the window stays in the
/// taskbar or the Dock, where it is seen and found again.
#[tauri::command]
pub fn minimize_main(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
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
    /// For Quit: the received sticks coming in now (they go on next time).
    receiving: Vec<String>,
    /// For Quit: received sticks are kept up to date, or waited for.
    following: bool,
}

/// What the dialog says: about `stick_id`, or (none) about every stick.
#[tauri::command]
pub async fn confirm_view(
    stick_id: Option<String>,
    state: State<'_, AppState>,
    receivers: State<'_, crate::receiving::Receivers>,
) -> Result<ConfirmView, String> {
    let all = state
        .sticks_now()
        .await
        .map_err(|error| crate::words::told(&error))?;
    let stick_id = stick_id.as_deref().map(stick_id_of).transpose()?;
    let mut view = confirm_of(&all, &*state.sticks.lock().await, stick_id);
    if stick_id.is_none() {
        view.following = receivers.following();
        let coming = receivers.downloading();
        let store = state.store.clone();
        view.receiving = crate::commands::blocking(move || {
            Ok(store
                .joined_sticks()?
                .into_iter()
                .filter(|joined| coming.contains(&joined.stick_id))
                .map(|joined| {
                    ourstick_core::received::display_name(joined.name.as_deref(), &joined.stick_id)
                })
                .collect())
        })
        .await?;
    }
    Ok(view)
}

/// What the dialog says about `stick_id` (or, with none, every stick).
fn confirm_of(
    all: &[ourstick_core::OwnedStick],
    sharing: &std::collections::HashMap<StickId, Sharing>,
    stick_id: Option<StickId>,
) -> ConfirmView {
    let readers_of = |id: &StickId| match sharing.get(id) {
        Some(Sharing::On(serving)) => serving.activity.borrow().readers,
        _ => 0,
    };
    let shared_now = |id: &StickId| sharing.get(id).is_some_and(Sharing::is_on);
    let names = |keep: &dyn Fn(&StickId) -> bool| -> Vec<String> {
        all.iter()
            .filter(|stick| keep(&stick.stick_id))
            .map(|stick| stick.name.clone())
            .collect()
    };
    match stick_id {
        Some(id) => ConfirmView {
            sharing: shared_now(&id),
            readers: readers_of(&id),
            name: all
                .iter()
                .find(|stick| stick.stick_id == id)
                .map(|stick| stick.name.clone()),
            shared: Vec::new(),
            downloading: Vec::new(),
            receiving: Vec::new(),
            following: false,
        },
        None => ConfirmView {
            sharing: sharing.values().any(Sharing::is_on),
            readers: readers_now(sharing),
            name: None,
            shared: names(&shared_now),
            downloading: names(&|id| readers_of(id) > 0),
            receiving: Vec::new(),
            following: false,
        },
    }
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
    open(&app, mode).map_err(|error| crate::words::told(&error))
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
        .map_err(|error| crate::words::told(&error))?;
    stop_serving(&state, &stick_id).await;
    refresh_tray(&app).await;
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
        .map_err(|error| crate::words::told(&error))?;
    // A busy stick is shared again, if wanted, when what keeps it busy ends.
    start_sharing_later(&app, stick_id);
    Ok(())
}
/// Set once the user confirmed Quit: any other way out (the Dock, ⌘Q, the
/// app menu) is held back and shows the dialog first (see main).
pub static QUIT_CONFIRMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set once quitting has begun: the app is stopping and about to go.
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Quitting has begun: the app is stopping and about to go.
pub fn quitting() -> bool {
    QUITTING.load(std::sync::atomic::Ordering::SeqCst)
}

/// While quitting, brings the Quit dialog ("Quitting… please wait.")
/// forward instead of the main window, which would show and vanish; true
/// then. For a launch, the tray or the Dock during the few seconds it takes.
pub fn quitting_shown(app: &AppHandle) -> bool {
    if !QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    for (label, window) in app.webview_windows() {
        if label.starts_with("confirm") {
            crate::windows::bring_forward(&window);
        }
    }
    true
}

/// Quits the app. Stopping takes a few seconds (the sticks' connections
/// close, a download finishes the file it places): it runs here, off the
/// main thread, so that no window looks stuck ("not responding" on GNOME).
/// Meanwhile only the Quit dialog stays, saying so. Main's own stop on Exit
/// then finds nothing left.
#[tauri::command]
pub async fn quit(app: AppHandle) {
    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
    for (label, window) in app.webview_windows() {
        if !label.starts_with("confirm") {
            let _ = window.hide();
        }
    }
    crate::sharing::stop_all(&app.state::<AppState>()).await;
    app.state::<crate::receiving::Receivers>().stop_all().await;
    QUIT_CONFIRMED.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn stick(byte: u8, name: &str) -> ourstick_core::OwnedStick {
        let id = ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id();
        ourstick_core::OwnedStick::new(id, name, "/s", ourstick_core::CipherId::None, 0)
    }

    #[test]
    fn quit_names_the_sticks_being_shared_oldest_first() {
        let all = [stick(1, "Photos"), stick(2, "Films"), stick(3, "Music")];
        let mut sharing = HashMap::new();
        sharing.insert(all[0].stick_id, Sharing::Starting);
        sharing.insert(all[1].stick_id, Sharing::Off);
        sharing.insert(all[2].stick_id, Sharing::Starting);
        let view = confirm_of(&all, &sharing, None);
        assert!(view.sharing);
        assert_eq!(view.shared, ["Photos", "Music"]);
        assert!(view.downloading.is_empty());
        assert_eq!(view.name, None);
    }

    #[test]
    fn stop_sharing_names_its_stick() {
        let all = [stick(1, "Photos"), stick(2, "Films")];
        let view = confirm_of(&all, &HashMap::new(), Some(all[1].stick_id));
        assert_eq!(view.name.as_deref(), Some("Films"));
        assert!(!view.sharing);
        let gone = stick(9, "Gone").stick_id;
        assert_eq!(confirm_of(&all, &HashMap::new(), Some(gone)).name, None);
    }
}
