//! The tray icon (docs/gui-spec.md, panel 3): always there while the app
//! runs, with a green dot while someone downloads, and the app's menu: each
//! stick with its own submenu (ADR 0009 §5), then "New stick…", "About
//! OurStick", the "Start at login" check (ADR 0024) and "Quit".
//!
//! What the menu holds is decided here once ([`menu_lines`]); each platform
//! only draws it. Linux has a StatusNotifierItem of its own (`linux`): it
//! receives the XDG activation token of each click, which on Wayland is what
//! lets "Open OurStick" bring the window to the front. Elsewhere Tauri's
//! tray (`native`).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod native;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(target_os = "linux"))]
use native as platform;

pub use platform::{create, set_active, set_sticks};

use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{stick_hex, stick_id, AppState};
use crate::{dialog, share};

/// One of the app's sticks, as the tray shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayStick {
    pub id: StickId,
    pub name: String,
    pub mounted: bool,
    pub sharing: bool,
    /// While busy, what is being done ("Renaming…"): nothing to choose.
    pub doing: Option<&'static str>,
}

/// What the tray menu's items do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Share(StickId),
    /// Eject, or Mount after an eject.
    Mount(StickId),
    /// Stop sharing (asks first).
    StopSharing(StickId),
    StartSharing(StickId),
    NewStick,
    About,
    /// The "Start at login" check, turned the other way.
    StartAtLogin,
    Quit,
}

impl Action {
    /// The item's id in a native menu.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn id(self) -> String {
        match self {
            Self::Open => "open".to_owned(),
            Self::Share(stick) => format!("share:{}", stick_hex(&stick)),
            Self::Mount(stick) => format!("mount:{}", stick_hex(&stick)),
            Self::StopSharing(stick) => format!("stop:{}", stick_hex(&stick)),
            Self::StartSharing(stick) => format!("start:{}", stick_hex(&stick)),
            Self::NewStick => "new-stick".to_owned(),
            Self::About => "about".to_owned(),
            Self::StartAtLogin => "start-at-login".to_owned(),
            Self::Quit => "quit".to_owned(),
        }
    }

    /// The action of a native menu item's id.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn from_id(id: &str) -> Option<Self> {
        match id.split_once(':') {
            None => match id {
                "open" => Some(Self::Open),
                "new-stick" => Some(Self::NewStick),
                "about" => Some(Self::About),
                "start-at-login" => Some(Self::StartAtLogin),
                "quit" => Some(Self::Quit),
                _ => None,
            },
            Some((what, hex)) => {
                let stick = stick_id(hex).ok()?;
                match what {
                    "share" => Some(Self::Share(stick)),
                    "mount" => Some(Self::Mount(stick)),
                    "stop" => Some(Self::StopSharing(stick)),
                    "start" => Some(Self::StartSharing(stick)),
                    _ => None,
                }
            }
        }
    }
}

/// A line of the tray menu.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    Item(String, Action),
    /// A stick: its label and its submenu.
    Stick(String, Vec<(String, Action)>),
    /// Says something; nothing to choose.
    Note(String),
    /// An item with a check mark, on or off.
    Check(String, bool, Action),
    Separator,
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

/// The whole tray menu, for the app's sticks (oldest first) and whether
/// the app starts at login.
fn menu_lines(sticks: &[TrayStick], start_at_login: bool) -> Vec<Line> {
    let mut lines = vec![Line::Item("Open OurStick".to_owned(), Action::Open)];
    if !sticks.is_empty() {
        lines.push(Line::Separator);
    }
    for stick in sticks {
        if let Some(doing) = stick.doing {
            lines.push(Line::Note(format!("{} \u{2014} {doing}", stick.name)));
            continue;
        }
        let state = if stick.sharing {
            "sharing"
        } else {
            "not sharing"
        };
        lines.push(Line::Stick(
            format!("{} \u{2014} {state}", stick.name),
            vec![
                ("Share…".to_owned(), Action::Share(stick.id)),
                (
                    mount_label(stick.mounted).to_owned(),
                    Action::Mount(stick.id),
                ),
                (
                    sharing_label(stick.sharing).to_owned(),
                    if stick.sharing {
                        Action::StopSharing(stick.id)
                    } else {
                        Action::StartSharing(stick.id)
                    },
                ),
            ],
        ));
    }
    lines.push(Line::Separator);
    lines.push(Line::Item("New stick…".to_owned(), Action::NewStick));
    lines.push(Line::Separator);
    lines.push(Line::Item("About OurStick".to_owned(), Action::About));
    lines.push(Line::Check(
        "Start at login".to_owned(),
        start_at_login,
        Action::StartAtLogin,
    ));
    lines.push(Line::Item("Quit".to_owned(), Action::Quit));
    lines
}

fn act(app: &AppHandle, action: Action) {
    // The keys' step first (ADR 0021): the window shows it. About and the
    // check touch no key.
    if crate::keys::waiting(&app.state::<AppState>())
        && !matches!(
            action,
            Action::Open | Action::Quit | Action::About | Action::StartAtLogin
        )
    {
        show_main(app);
        return;
    }
    let result = match action {
        Action::Open => {
            show_main(app);
            Ok(())
        }
        Action::Share(stick) => share::open(app, &stick),
        Action::Mount(stick) => {
            toggle_mount(app, stick);
            Ok(())
        }
        Action::StopSharing(stick) => dialog::open(app, dialog::Mode::StopSharing(stick)),
        Action::StartSharing(stick) => {
            start(app, stick);
            Ok(())
        }
        Action::NewStick => {
            show_main(app);
            // The window asks for the new stick's name.
            app.emit_to("main", "new-stick", ())
                .map_err(anyhow::Error::from)
        }
        Action::About => crate::about::open(app),
        Action::StartAtLogin => {
            toggle_start_at_login(app);
            Ok(())
        }
        Action::Quit => dialog::open(app, dialog::Mode::Quit),
    };
    if let Err(error) = result {
        tracing::warn!(error = %format!("{error:#}"), "tray menu action failed");
    }
}

/// "Start at login" turned the other way from what the check shows.
fn toggle_start_at_login(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let store = app.state::<AppState>().store.clone();
        let changed = tauri::async_runtime::spawn_blocking(move || {
            let on = crate::login::shown(store.as_ref());
            crate::login::set_by_user(store.as_ref(), !on)
        })
        .await;
        if let Ok(Err(error)) = changed {
            tracing::warn!(error = %format!("{error:#}"), "start at login was not changed");
            crate::problems::failed(&app, None, crate::words::told(&error));
        }
        crate::login::commands::changed(&app).await;
    });
}

/// Shows the main window again (from the tray, or a second launch).
pub fn show_main(app: &AppHandle) {
    if crate::dialog::quitting_shown(app) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        crate::windows::bring_forward(&window);
    }
}

/// Tells the window about a problem of `stick_id` it cannot otherwise see.
fn problem(app: &AppHandle, stick_id: StickId, message: String) {
    crate::problems::failed(app, Some(stick_id), message);
}

fn toggle_mount(app: &AppHandle, stick_id: StickId) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if crate::sharing::is_busy(&state, &stick_id).await {
            return problem(
                &app,
                stick_id,
                "The stick is busy; try again in a moment.".to_owned(),
            );
        }
        let stick = match state.stick_now(&stick_id).await {
            Ok(stick) => stick,
            Err(error) => return problem(&app, stick_id, crate::words::told(&error)),
        };
        // The disk is read and `hdiutil`/`subst` run off the async runtime.
        let Ok(ejected) = tauri::async_runtime::spawn_blocking(move || {
            crate::drive::is_mounted(&stick).then(|| crate::drive::eject(&stick))
        })
        .await
        else {
            return;
        };
        match ejected {
            Some(ejected) => {
                if let Err(error) = ejected {
                    tracing::warn!(error = %format!("{error:#}"), "eject failed");
                    problem(&app, stick_id, crate::words::told(&error));
                }
                let _ = app.emit("mount-changed", ());
                crate::sharing::refresh_tray(&app).await;
            }
            None => crate::sharing::mount_and_share(&app, stick_id),
        }
    });
}

/// "Start sharing": wanted shared from now on, then shared; a busy stick
/// says so.
fn start(app: &AppHandle, stick_id: StickId) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if crate::sharing::is_busy(&state, &stick_id).await {
            return problem(
                &app,
                stick_id,
                "The stick is busy; try again in a moment.".to_owned(),
            );
        }
        match state.set_sharing_wanted(&stick_id, true).await {
            Ok(()) => crate::sharing::start_sharing(&app, stick_id).await,
            Err(error) => problem(&app, stick_id, crate::words::told(&error)),
        }
    });
}

#[cfg(test)]
mod tests;
