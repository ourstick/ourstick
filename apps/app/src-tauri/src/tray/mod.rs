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

/// The stick the tray's items act on: the app's first, until the tray
/// lists every stick (ADR 0009 §5, step 7a-iii).
fn first_stick(app: &AppHandle) -> Option<ourstick_core::OwnedStick> {
    let state = app.state::<crate::state::AppState>();
    state.app_sticks().ok()?.into_iter().next()
}

fn act(app: &AppHandle, action: Action) {
    let result = match action {
        Action::Open => {
            show_main(app);
            Ok(())
        }
        Action::Share => match first_stick(app) {
            Some(stick) => share::open(app, &stick.stick_id),
            None => Ok(()),
        },
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
        // The disk is read and `hdiutil`/`subst` run off the async runtime.
        let handle = app.clone();
        let Ok(Some(stick)) =
            tauri::async_runtime::spawn_blocking(move || first_stick(&handle)).await
        else {
            return;
        };
        let stick_id = stick.stick_id;
        let state = app.state::<crate::state::AppState>();
        if crate::sharing::is_busy(&state, &stick_id).await {
            let _ = tauri::Emitter::emit(
                &app,
                "share-problem",
                crate::serving::ShareProblem::Failed {
                    message: "The stick is busy; try again in a moment.".to_owned(),
                },
            );
            return;
        }
        let Ok(ejected) = tauri::async_runtime::spawn_blocking(move || {
            crate::drive::is_mounted(&stick).then(|| crate::drive::eject(&stick))
        })
        .await
        else {
            return;
        };
        if let Some(ejected) = ejected {
            if let Err(error) = ejected {
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
            crate::sharing::mount_and_share(&app, stick_id);
        }
    });
}

fn toggle_sharing(app: &AppHandle) -> Result<()> {
    let Some(stick) = first_stick(app) else {
        return Ok(());
    };
    let state = app.state::<crate::state::AppState>();
    // Never wait on the main thread: a busy lock means sharing is changing,
    // so ask as if it were on.
    let sharing = state.sticks.try_lock().map_or(true, |sticks| {
        sticks
            .get(&stick.stick_id)
            .is_some_and(crate::state::Sharing::is_on)
    });
    if sharing {
        dialog::open(app, dialog::Mode::StopSharing(stick.stick_id))
    } else {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<crate::state::AppState>();
            match state.set_sharing_wanted(&stick.stick_id, true).await {
                Ok(()) => crate::sharing::start_sharing(&app, stick.stick_id, 1).await,
                Err(error) => tracing::warn!(error = %format!("{error:#}"), "cannot start sharing"),
            }
        });
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
