//! Showing the app's windows in front. On Linux (GNOME on Wayland) an app may
//! raise its own window only with an XDG activation token from the user's
//! click; without one GNOME shows an "OurStick is ready" notification
//! instead. The tray receives a token for each click on it (tray/linux.rs).

use tauri::{Runtime, WebviewWindow};

/// Whether new windows ask for focus when they open. On Linux they do not:
/// GNOME answers a request without a token with the notification.
#[cfg(desktop)]
pub const FOCUS_NEW: bool = !cfg!(target_os = "linux");

/// How long a token is used after it came. A token belongs to one click; a
/// later action gets none rather than an old one.
#[cfg(target_os = "linux")]
const TOKEN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(2);

/// The latest XDG activation token, from the tray, until a window uses it.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub struct Activation(std::sync::Mutex<Option<(String, std::time::Instant)>>);

#[cfg(target_os = "linux")]
impl Activation {
    pub fn provide(&self, token: String) {
        *self.0.lock().expect("activation lock") = Some((token, std::time::Instant::now()));
    }

    /// Whether a token came at or after `since`.
    pub fn received_since(&self, since: std::time::Instant) -> bool {
        self.0
            .lock()
            .expect("activation lock")
            .as_ref()
            .is_some_and(|(_, received)| *received >= since)
    }

    /// The token, if it is fresh; used once.
    fn take(&self) -> Option<String> {
        self.0
            .lock()
            .expect("activation lock")
            .take()
            .filter(|(_, received)| received.elapsed() < TOKEN_LIFETIME)
            .map(|(token, _)| token)
    }
}

/// A window of its own: About, or a dialog (over the others, centred).
#[cfg_attr(mobile, allow(dead_code))]
pub struct OwnWindow<'a> {
    pub label: &'a str,
    /// The page, e.g. `index.html#about`.
    pub page: String,
    pub title: &'a str,
    pub size: (f64, f64),
    pub dialog: bool,
}

/// Makes `own`, not resizable. A phone shows everything in its one window
/// (ADR 0027).
#[cfg(desktop)]
pub fn own_window(app: &tauri::AppHandle, own: OwnWindow) -> anyhow::Result<WebviewWindow> {
    let builder =
        tauri::WebviewWindowBuilder::new(app, own.label, tauri::WebviewUrl::App(own.page.into()))
            .title(own.title)
            .focused(FOCUS_NEW)
            .inner_size(own.size.0, own.size.1)
            .resizable(false);
    let builder = if own.dialog {
        builder.always_on_top(true).center()
    } else {
        builder
    };
    Ok(builder.build()?)
}

/// See the desktop's [`own_window`].
#[cfg(mobile)]
pub fn own_window(_app: &tauri::AppHandle, _own: OwnWindow) -> anyhow::Result<WebviewWindow> {
    anyhow::bail!("a phone has no windows of their own")
}

/// Brings a window just made in front of the others, focused.
#[cfg(not(target_os = "linux"))]
pub fn show_new<R: Runtime>(window: &WebviewWindow<R>) {
    bring_forward(window);
}

/// Shows `window` (again) in front of the others, focused.
#[cfg(not(target_os = "linux"))]
pub fn bring_forward<R: Runtime>(window: &WebviewWindow<R>) {
    if !window.is_visible().unwrap_or(false) {
        let _ = window.show();
    }
    #[cfg(desktop)]
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.set_focus();
}

/// The tray click's token, if there is a fresh one.
#[cfg(target_os = "linux")]
fn token<R: Runtime>(window: &WebviewWindow<R>) -> Option<String> {
    use tauri::Manager;

    window
        .try_state::<Activation>()
        .and_then(|activation| activation.take())
}

/// Runs `show` with the window's GTK window, on the main thread.
#[cfg(target_os = "linux")]
fn with_gtk<R: Runtime>(
    window: &WebviewWindow<R>,
    show: impl FnOnce(gtk::ApplicationWindow) + Send + 'static,
) {
    let target = window.clone();
    let queued = window.run_on_main_thread(move || match target.gtk_window() {
        Ok(gtk) => show(gtk),
        Err(error) => tracing::warn!(%error, "cannot bring the window forward"),
    });
    if let Err(error) = queued {
        tracing::warn!(%error, "cannot bring the window forward");
    }
}

/// A window just made, on Linux: nothing to do. GNOME focuses a new window
/// of the app that has the focus, and only then (see `FOCUS_NEW`); the tray
/// brings the main window forward first (tray/linux.rs). Asking here would
/// not help: GTK passes a token before GNOME has the new window, which loses
/// it, and present() without one asks for a token with the app's last
/// click, which GNOME refuses after a click on the tray.
#[cfg(target_os = "linux")]
pub fn show_new<R: Runtime>(_window: &WebviewWindow<R>) {}

/// Shows `window` in front of the others, with the tray click's token if
/// there is one: on a window GNOME already has on screen (also when
/// minimized) GTK passes the token at once. present() then raises it, and
/// restores it if minimized; Tauri's show() and set_focus() would ask
/// without the token first.
///
/// The main window is only ever minimized, never hidden (see main): GTK 3
/// makes a hidden window anew when it is shown, and GNOME then sees a new
/// window without the token. It is hidden only until its first show, when
/// started at login.
#[cfg(target_os = "linux")]
pub fn bring_forward<R: Runtime>(window: &WebviewWindow<R>) {
    use gtk::prelude::{GtkWindowExt, WidgetExt};

    let token = token(window);
    with_gtk(window, move |gtk| {
        if let Some(token) = &token {
            gtk.set_startup_id(token);
        }
        // Never shown yet (started at login): tao shows the box that holds
        // the page only in its own show, so present() alone would map an
        // empty, white window. In the GTK call itself, before present():
        // Tauri's show() goes through the event loop and could come after.
        if !gtk.is_visible() {
            gtk.show_all();
        }
        gtk.present();
    });
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_token_is_used_once() {
        let activation = Activation::default();
        let before = Instant::now();
        assert!(!activation.received_since(before));
        activation.provide("token".into());
        assert!(activation.received_since(before));
        assert_eq!(activation.take().as_deref(), Some("token"));
        assert_eq!(activation.take(), None);
    }

    #[test]
    fn an_old_token_is_not_used() {
        let activation = Activation::default();
        let old = Instant::now()
            .checked_sub(TOKEN_LIFETIME + Duration::from_millis(1))
            .unwrap();
        *activation.0.lock().unwrap() = Some(("token".into(), old));
        assert!(!activation.received_since(Instant::now()));
        assert_eq!(activation.take(), None);
    }
}
