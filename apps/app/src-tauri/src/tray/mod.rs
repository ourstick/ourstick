//! The tray icon (docs/gui-spec.md, panel 3): always there while the app
//! runs, with a green dot while someone downloads, and the app's menu: each
//! stick with its own submenu (ADR 0009 §5), then "New stick…" and "Quit".
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

use anyhow::Result;
use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{stick_hex, stick_id, AppState, Sharing};
use crate::{dialog, share};

/// One of the app's sticks, as the tray shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayStick {
    pub id: StickId,
    pub name: String,
    pub mounted: bool,
    pub sharing: bool,
}

/// What the tray menu's items do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Share(StickId),
    /// Eject, or Mount after an eject.
    Mount(StickId),
    /// Stop sharing (asks first), or Start sharing after a stop.
    Sharing(StickId),
    NewStick,
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
            Self::Sharing(stick) => format!("sharing:{}", stick_hex(&stick)),
            Self::NewStick => "new-stick".to_owned(),
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
                "quit" => Some(Self::Quit),
                _ => None,
            },
            Some((what, hex)) => {
                let stick = stick_id(hex).ok()?;
                match what {
                    "share" => Some(Self::Share(stick)),
                    "mount" => Some(Self::Mount(stick)),
                    "sharing" => Some(Self::Sharing(stick)),
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

/// The whole tray menu, for the app's sticks (oldest first).
fn menu_lines(sticks: &[TrayStick]) -> Vec<Line> {
    let mut lines = vec![Line::Item("Open OurStick".to_owned(), Action::Open)];
    if !sticks.is_empty() {
        lines.push(Line::Separator);
    }
    for stick in sticks {
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
                    Action::Sharing(stick.id),
                ),
            ],
        ));
    }
    lines.push(Line::Separator);
    lines.push(Line::Item("New stick…".to_owned(), Action::NewStick));
    lines.push(Line::Item("Quit".to_owned(), Action::Quit));
    lines
}

fn act(app: &AppHandle, action: Action) {
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
        Action::Sharing(stick) => toggle_sharing(app, stick),
        Action::NewStick => {
            show_main(app);
            // The window asks for the new stick's name.
            app.emit_to("main", "new-stick", ())
                .map_err(anyhow::Error::from)
        }
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

/// Tells the window about a problem it cannot otherwise see.
fn problem(app: &AppHandle, message: String) {
    let _ = app.emit(
        "share-problem",
        crate::serving::ShareProblem::Failed { message },
    );
}

fn toggle_mount(app: &AppHandle, stick_id: StickId) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if crate::sharing::is_busy(&state, &stick_id).await {
            return problem(&app, "The stick is busy; try again in a moment.".to_owned());
        }
        let stick = match state.stick_now(&stick_id).await {
            Ok(stick) => stick,
            Err(error) => return problem(&app, format!("{error:#}")),
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
                    problem(&app, format!("{error:#}"));
                }
                let _ = app.emit("mount-changed", ());
                crate::sharing::refresh_tray(&app).await;
            }
            None => crate::sharing::mount_and_share(&app, stick_id),
        }
    });
}

fn toggle_sharing(app: &AppHandle, stick_id: StickId) -> Result<()> {
    let state = app.state::<AppState>();
    // Never wait on the main thread: a busy lock means sharing is changing,
    // so ask as if it were on.
    let sharing = state.sticks.try_lock().map_or(true, |sticks| {
        sticks.get(&stick_id).is_some_and(Sharing::is_on)
    });
    if sharing {
        dialog::open(app, dialog::Mode::StopSharing(stick_id))
    } else {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            match state.set_sharing_wanted(&stick_id, true).await {
                Ok(()) => crate::sharing::start_sharing(&app, stick_id, 1).await,
                Err(error) => problem(&app, format!("{error:#}")),
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stick(byte: u8, name: &str, mounted: bool, sharing: bool) -> TrayStick {
        TrayStick {
            id: ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id(),
            name: name.to_owned(),
            mounted,
            sharing,
        }
    }

    #[test]
    fn the_toggling_items_say_what_they_will_do() {
        assert_eq!(mount_label(true), "Eject");
        assert_eq!(mount_label(false), "Mount");
        assert_eq!(sharing_label(true), "Stop sharing…");
        assert_eq!(sharing_label(false), "Start sharing");
    }

    #[test]
    fn each_stick_has_its_own_submenu() {
        let photos = stick(1, "Photos", true, true);
        let films = stick(2, "Films", false, false);
        let lines = menu_lines(&[photos.clone(), films.clone()]);
        assert_eq!(lines.len(), 7);
        assert_eq!(
            lines[2],
            Line::Stick(
                "Photos \u{2014} sharing".to_owned(),
                vec![
                    ("Share…".to_owned(), Action::Share(photos.id)),
                    ("Eject".to_owned(), Action::Mount(photos.id)),
                    ("Stop sharing…".to_owned(), Action::Sharing(photos.id)),
                ]
            )
        );
        let Line::Stick(label, items) = &lines[3] else {
            panic!("{:?}", lines[3]);
        };
        assert_eq!(label, "Films \u{2014} not sharing");
        assert_eq!(items[1].0, "Mount");
        assert_eq!(items[2].0, "Start sharing");
        assert_eq!(
            lines[5],
            Line::Item("New stick…".to_owned(), Action::NewStick)
        );
    }

    #[test]
    fn with_no_stick_the_menu_still_opens_makes_and_quits() {
        let lines = menu_lines(&[]);
        assert_eq!(
            lines[0],
            Line::Item("Open OurStick".to_owned(), Action::Open)
        );
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_native_item_id_names_its_action() {
        let id = stick(3, "Photos", true, true).id;
        for action in [
            Action::Open,
            Action::Share(id),
            Action::Mount(id),
            Action::Sharing(id),
            Action::NewStick,
            Action::Quit,
        ] {
            assert_eq!(Action::from_id(&action.id()), Some(action));
        }
        assert_eq!(Action::from_id("share:zz"), None);
        assert_eq!(Action::from_id("eject"), None);
    }
}
