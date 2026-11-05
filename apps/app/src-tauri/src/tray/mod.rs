//! The tray icon (ADR 0028 §6): always there while the app runs, a green
//! dot while someone gets something, and the app's menu: Open OurStick;
//! each thing sent or received with its one sentence, a click opening the
//! window on it; Send…, Receive…; About OurStick, the "Open when the
//! computer starts" check (ADR 0024) and Quit…. A help, never the only way.
//!
//! What the menu holds is decided here once ([`menu_lines`]); each platform
//! only draws it. Linux has a StatusNotifierItem of its own (`linux`): it
//! receives the XDG activation token of each click, which on Wayland is what
//! lets "Open OurStick" bring the window to the front. Elsewhere Tauri's
//! tray (`native`).
//!
//! A phone has no tray (ADR 0027): the menu is built there, unused.
#![cfg_attr(mobile, allow(dead_code))]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(desktop, not(target_os = "linux")))]
mod native;
#[cfg(mobile)]
mod phone;
pub mod words;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(all(desktop, not(target_os = "linux")))]
use native as platform;
#[cfg(mobile)]
use phone as platform;

pub use platform::{create, set_sticks};

use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::dialog;
use crate::state::{stick_hex, stick_id, AppState};

/// The green dot as last drawn. A download changes its speed twice a
/// second; the tray is drawn again only when the dot itself changes.
#[derive(Default)]
pub struct DotShown(std::sync::Mutex<Option<bool>>);

impl DotShown {
    /// Whether `active` differs from what is drawn; it is drawn from now on.
    fn changes_to(&self, active: bool) -> bool {
        self.0.lock().expect("tray dot lock").replace(active) != Some(active)
    }
}

/// The green dot, while someone downloads.
pub fn set_active(app: &AppHandle, active: bool) {
    let changed = app
        .try_state::<DotShown>()
        .is_none_or(|dot| dot.changes_to(active));
    if changed {
        platform::set_active(app, active);
    }
}

/// Something sent or received, as the tray shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayThing {
    pub shows: Shows,
    pub name: String,
    /// Its one sentence, with its symbol ("↑ Someone is getting it").
    pub words: String,
}

/// Which view of the window a thing opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shows {
    Sending(StickId),
    Received(StickId),
}

/// What the tray menu's items do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    /// The window, on one thing.
    Show(Shows),
    Send,
    Receive,
    About,
    /// The line of a new version (ADR 0033): the download page.
    GetIt,
    /// The "Open when the computer starts" check, turned the other way.
    StartAtLogin,
    Quit,
}

impl Action {
    /// The item's id in a native menu.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn id(self) -> String {
        match self {
            Self::Open => "open".to_owned(),
            Self::Show(Shows::Sending(stick)) => format!("sending:{}", stick_hex(&stick)),
            Self::Show(Shows::Received(stick)) => format!("received:{}", stick_hex(&stick)),
            Self::Send => "send".to_owned(),
            Self::Receive => "receive".to_owned(),
            Self::About => "about".to_owned(),
            Self::GetIt => "get-it".to_owned(),
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
                "send" => Some(Self::Send),
                "receive" => Some(Self::Receive),
                "about" => Some(Self::About),
                "get-it" => Some(Self::GetIt),
                "start-at-login" => Some(Self::StartAtLogin),
                "quit" => Some(Self::Quit),
                _ => None,
            },
            Some((what, hex)) => {
                let stick = stick_id(hex).ok()?;
                match what {
                    "sending" => Some(Self::Show(Shows::Sending(stick))),
                    "received" => Some(Self::Show(Shows::Received(stick))),
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
    /// An item with a check mark, on or off.
    Check(String, bool, Action),
    Separator,
}

/// The whole tray menu, for what is sent and received (in the home's
/// order), whether the app starts at login, and a new version's line.
fn menu_lines(things: &[TrayThing], start_at_login: bool, notice: Option<&str>) -> Vec<Line> {
    let mut lines = vec![Line::Item("Open OurStick".to_owned(), Action::Open)];
    if !things.is_empty() {
        lines.push(Line::Separator);
    }
    for thing in things {
        lines.push(Line::Item(
            format!("{} \u{2014} {}", thing.name, thing.words),
            Action::Show(thing.shows),
        ));
    }
    lines.push(Line::Separator);
    lines.push(Line::Item("Send…".to_owned(), Action::Send));
    lines.push(Line::Item("Receive…".to_owned(), Action::Receive));
    lines.push(Line::Separator);
    if let Some(notice) = notice {
        lines.push(Line::Item(notice.to_owned(), Action::GetIt));
    }
    lines.push(Line::Item("About OurStick".to_owned(), Action::About));
    lines.push(Line::Check(
        "Open when the computer starts".to_owned(),
        start_at_login,
        Action::StartAtLogin,
    ));
    lines.push(Line::Item("Quit…".to_owned(), Action::Quit));
    lines
}

/// What the window is asked to show: a thing's view, Send or Receive.
#[derive(Debug, Clone, serde::Serialize)]
struct WindowShows {
    kind: &'static str,
    id: Option<String>,
}

fn act(app: &AppHandle, action: Action) {
    // The keys' step first (ADR 0021): the window shows it. About and the
    // check touch no key.
    if crate::keys::waiting(&app.state::<AppState>())
        && !matches!(
            action,
            Action::Open | Action::Quit | Action::About | Action::GetIt | Action::StartAtLogin
        )
    {
        show_main(app);
        return;
    }
    let shows = |kind, id: Option<&StickId>| {
        show_main(app);
        app.emit_to(
            "main",
            "show",
            WindowShows {
                kind,
                id: id.map(stick_hex),
            },
        )
        .map_err(anyhow::Error::from)
    };
    let result = match action {
        Action::Open => {
            show_main(app);
            Ok(())
        }
        Action::Show(Shows::Sending(stick)) => shows("sending", Some(&stick)),
        Action::Show(Shows::Received(stick)) => shows("received", Some(&stick)),
        Action::Send => shows("send", None),
        Action::Receive => shows("receive", None),
        Action::About => crate::about::open(app),
        Action::GetIt => {
            crate::new_version::open_download_page_from_tray(app);
            Ok(())
        }
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

#[cfg(test)]
mod tests;
