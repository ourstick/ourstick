//! The tray icon (docs/gui-spec.md, panel 3): always there while the app
//! runs, with a green dot while someone downloads, and the app's menu.
//!
//! Linux has a StatusNotifierItem of its own (`linux`): it receives the XDG
//! activation token of each click, which on Wayland is what lets "Open
//! OurStick" bring the window to the front. Elsewhere Tauri's tray (`native`).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod native;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(target_os = "linux"))]
use native as platform;

pub use platform::{create, set_active, set_mounted, set_sharing};

use anyhow::Result;
use tauri::{AppHandle, Manager};

use crate::{dialog, share};

/// What the tray menu's items do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Share,
    /// Eject, or Mount after an eject.
    Mount,
    /// Stop sharing (asks first), or Start sharing after a stop.
    Sharing,
    Quit,
}

/// The Eject / Mount item's text.
fn mount_label(mounted: bool) -> &'static str {
    if mounted {
        "Eject"
    } else {
        "Mount"
    }
}

/// The Stop sharing / Start sharing item's text.
fn sharing_label(sharing: bool) -> &'static str {
    if sharing {
        "Stop sharing…"
    } else {
        "Start sharing"
    }
}

fn act(app: &AppHandle, action: Action) {
    let result = match action {
        Action::Open => {
            show_main(app);
            Ok(())
        }
        Action::Share => share::open(app),
        Action::Mount => {
            toggle_mount(app);
            Ok(())
        }
        Action::Sharing => toggle_sharing(app),
        Action::Quit => dialog::open(app, dialog::Mode::Quit),
    };
    if let Err(error) = result {
        tracing::warn!(error = %format!("{error:#}"), "tray menu action failed");
    }
}

/// Shows the main window again (from the tray, or a second launch).
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        crate::windows::bring_forward(&window);
    }
}

fn toggle_mount(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::state::AppState>();
        let Ok(Some(stick)) = state.drive_stick() else {
            return;
        };
        if crate::drive::is_mounted(&stick.root_path) {
            if let Err(error) = crate::drive::eject(&stick.root_path) {
                tracing::warn!(error = %format!("{error:#}"), "eject failed");
                let _ = tauri::Emitter::emit(
                    &app,
                    "share-problem",
                    crate::serving::ShareProblem::Failed {
                        message: format!("{error:#}"),
                    },
                );
            }
            set_mounted(&app, false);
            let _ = tauri::Emitter::emit(&app, "mount-changed", ());
        } else {
            crate::state::plug_in_and_share(&app);
        }
    });
}

fn toggle_sharing(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::state::AppState>();
    // Never wait on the main thread: a busy lock means sharing is changing,
    // so ask as if it were on.
    let sharing = state.sharing.try_lock().map_or(true, |sharing| {
        matches!(
            *sharing,
            crate::state::Sharing::On(_) | crate::state::Sharing::Starting
        )
    });
    if sharing {
        dialog::open(app, dialog::Mode::StopSharing)
    } else {
        crate::state::start_sharing(app);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toggling_items_say_what_they_will_do() {
        assert_eq!(mount_label(true), "Eject");
        assert_eq!(mount_label(false), "Mount");
        assert_eq!(sharing_label(true), "Stop sharing…");
        assert_eq!(sharing_label(false), "Start sharing");
    }
}
