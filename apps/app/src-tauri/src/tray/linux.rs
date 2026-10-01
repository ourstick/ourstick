//! The Linux tray: a StatusNotifierItem of our own (ksni), not Tauri's
//! (libayatana-appindicator). GNOME's AppIndicator extension passes an XDG
//! activation token before each click; only with it may the app bring a
//! window to the front on Wayland (see `crate::windows`).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use ksni::TrayMethods;
use tauri::{AppHandle, Manager};

use super::{act, menu_lines, Action, Line, TrayStick};
use crate::windows::Activation;

/// How long a click waits for its token. The extension does not wait for
/// our reply before sending the click, so the token comes a little after it
/// (about 10 ms on Ubuntu 24.04).
const TOKEN_WAIT: Duration = Duration::from_millis(300);

/// How long a new window from the tray waits for the main window to get the
/// focus.
const FOCUS_WAIT: Duration = Duration::from_millis(500);

/// What the tray shows; kept here so that changes made before the tray is
/// up are not lost.
#[derive(Debug, Clone)]
struct Shown {
    /// Someone downloads: the green dot.
    active: bool,
    sticks: Vec<TrayStick>,
}

/// The running tray, as app state.
struct LinuxTray {
    shown: Mutex<Shown>,
    handle: OnceLock<ksni::Handle<Tray>>,
}

struct Tray {
    app: AppHandle,
    /// The green dot, laid over the icon while someone downloads.
    dot: Vec<ksni::Icon>,
}

/// The icon theme's USB drive: it follows the panel's theme and size. Stands
/// in until OurStick has its own tray icon.
const ICON_NAME: &str = "drive-removable-media-usb";

/// Size of the overlay, in pixels; the panel scales it to the icon.
const DOT_SIZE: usize = 32;

/// The green of the dot (#2EA44F, as on macOS).
const DOT_GREEN: [u8; 3] = [0x2e, 0xa4, 0x4f];

/// A green dot in the bottom right corner, as a StatusNotifierItem pixmap
/// (ARGB32, big-endian), with a dark rim so that it reads on any icon.
fn green_dot() -> Vec<ksni::Icon> {
    let centre = DOT_SIZE as f32 * 0.75;
    let radius = DOT_SIZE as f32 * 0.22;
    let rim = 1.5;
    let mut data = Vec::with_capacity(DOT_SIZE * DOT_SIZE * 4);
    for y in 0..DOT_SIZE {
        for x in 0..DOT_SIZE {
            let distance =
                ((x as f32 + 0.5 - centre).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
            // Soft edge over one pixel.
            let coverage = (radius + rim - distance + 0.5).clamp(0.0, 1.0);
            let alpha = (coverage * 255.0).round() as u8;
            let [red, green, blue] = if distance <= radius {
                DOT_GREEN
            } else {
                [0x1a, 0x1a, 0x1a]
            };
            data.extend_from_slice(&[alpha, red, green, blue]);
        }
    }
    vec![ksni::Icon {
        width: DOT_SIZE as i32,
        height: DOT_SIZE as i32,
        data,
    }]
}

pub fn create(app: &AppHandle) -> Result<()> {
    app.manage(LinuxTray {
        shown: Mutex::new(Shown {
            active: false,
            sticks: Vec::new(),
        }),
        handle: OnceLock::new(),
    });
    let tray = Tray {
        app: app.clone(),
        dot: green_dot(),
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match tray.spawn().await {
            Ok(handle) => {
                let state = app.state::<LinuxTray>();
                let _ = state.handle.set(handle);
                // Show what changed while the tray was starting.
                refresh(&app);
            }
            Err(error) => tracing::warn!(
                %error,
                "no tray icon: this desktop shows no StatusNotifierItem \
                 (on GNOME it needs the AppIndicator extension)"
            ),
        }
    });
    Ok(())
}

fn change(app: &AppHandle, edit: impl FnOnce(&mut Shown)) {
    let Some(state) = app.try_state::<LinuxTray>() else {
        return;
    };
    edit(&mut state.shown.lock().expect("tray state lock"));
    refresh(app);
}

/// Has the panel read the tray again.
fn refresh(app: &AppHandle) {
    let Some(handle) = app
        .try_state::<LinuxTray>()
        .and_then(|state| state.handle.get().cloned())
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        handle.update(|_| ()).await;
    });
}

/// The green dot, while someone downloads.
pub fn set_active(app: &AppHandle, active: bool) {
    change(app, |shown| shown.active = active);
}

/// The app's sticks, as the menu lists them.
pub fn set_sticks(app: &AppHandle, sticks: Vec<TrayStick>) {
    change(app, |shown| shown.sticks = sticks);
}

impl Tray {
    fn shown(&self) -> Shown {
        self.app
            .state::<LinuxTray>()
            .shown
            .lock()
            .expect("tray state lock")
            .clone()
    }

    /// Runs `action` once the click's token is in, or `TOKEN_WAIT` has
    /// passed; off the tray's task, which delivers the token.
    fn clicked(&self, action: Action) {
        let app = self.app.clone();
        let clicked = Instant::now();
        // A host may also send the token just before the click.
        let since = clicked
            .checked_sub(Duration::from_millis(500))
            .unwrap_or(clicked);
        std::thread::spawn(move || {
            let activation = app.state::<Activation>();
            while !activation.received_since(since) && clicked.elapsed() < TOKEN_WAIT {
                std::thread::sleep(Duration::from_millis(5));
            }
            // The token raises one window that GNOME already has; a window
            // made now would come too late for it (see windows::show_new).
            // So the main window comes forward with it first, and the new
            // one then opens in an app that has the focus.
            if opens_window(&app, action) {
                super::show_main(&app);
                wait_for_main_focus(&app);
            }
            act(&app, action);
        });
    }
}

/// Whether `action` makes a new window.
fn opens_window(app: &AppHandle, action: Action) -> bool {
    match action {
        Action::Share(stick) => app
            .get_webview_window(&format!("share-{}", crate::state::stick_hex(&stick)))
            .is_none(),
        // The Stop sharing / Quit dialog is always made anew.
        Action::StopSharing(_) | Action::Quit => true,
        Action::Open | Action::Mount(_) | Action::StartSharing(_) | Action::NewStick => false,
    }
}

/// Waits until the main window has the focus, for at most `FOCUS_WAIT`.
fn wait_for_main_focus(app: &AppHandle) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let started = Instant::now();
    while !main.is_focused().unwrap_or(false) && started.elapsed() < FOCUS_WAIT {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A label as the panel shows it: `_` marks an access key there, so a
/// stick's own `_` is doubled.
fn label(text: &str) -> String {
    text.replace('_', "__")
}

fn item(text: &str, action: Action) -> ksni::MenuItem<Tray> {
    ksni::menu::StandardItem {
        label: label(text),
        activate: Box::new(move |tray: &mut Tray| tray.clicked(action)),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    /// A click on the icon opens the menu, as on macOS.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "ourstick".into()
    }

    fn title(&self) -> String {
        "OurStick".into()
    }

    fn icon_name(&self) -> String {
        ICON_NAME.into()
    }

    fn overlay_icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.shown().active {
            self.dot.clone()
        } else {
            Vec::new()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "OurStick".into(),
            ..Default::default()
        }
    }

    fn provide_xdg_activation_token(&mut self, token: String) {
        self.app.state::<Activation>().provide(token);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        menu_lines(&self.shown().sticks)
            .into_iter()
            .map(|line| match line {
                Line::Item(text, action) => item(&text, action),
                Line::Stick(text, items) => ksni::menu::SubMenu {
                    label: label(&text),
                    submenu: items
                        .iter()
                        .map(|(text, action)| item(text, *action))
                        .collect(),
                    ..Default::default()
                }
                .into(),
                Line::Note(text) => ksni::menu::StandardItem {
                    label: label(&text),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                Line::Separator => ksni::MenuItem::Separator,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_name_keeps_its_underscores() {
        assert_eq!(label("my_photos"), "my__photos");
        assert_eq!(label("Φωτογραφίες"), "Φωτογραφίες");
    }

    #[test]
    fn the_dot_is_green_in_the_corner_and_clear_elsewhere() {
        let dot = &green_dot()[0];
        assert_eq!(dot.data.len(), DOT_SIZE * DOT_SIZE * 4);
        let pixel = |x: usize, y: usize| {
            let at = (y * DOT_SIZE + x) * 4;
            &dot.data[at..at + 4]
        };
        // ARGB: opaque green at the dot's centre, clear at the top left.
        assert_eq!(pixel(24, 24), &[255, 0x2e, 0xa4, 0x4f]);
        assert_eq!(pixel(2, 2)[0], 0);
    }
}
