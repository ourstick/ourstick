//! `ourstick:receive` opened by a browser (ADR 0030 §1): the app comes to
//! the front and Receive opens with nothing in it, to read the clipboard.
//! On Linux and Windows the address is an argument, of this start or of a
//! second start the single-instance plugin hands over; on macOS it comes as
//! `RunEvent::Opened`. The address is never logged: any other is a
//! stranger's text, and may hold a key.

use std::time::Instant;

use tauri::{AppHandle, Manager, Runtime};

use crate::receiving::scheme::{self, Arrivals};

/// Whether this start was asked for `ourstick:receive`.
pub fn asked_at_start() -> bool {
    scheme::asked(std::env::args_os())
}

/// Before Tauri starts, for a start that asks for `ourstick:receive`: on
/// Linux the app already running, if any, gets the activation token the
/// browser gave this start (GNOME raises a window only with one, and the
/// single-instance plugin hands over only the arguments). Windows needs
/// nothing: Tauri's `set_focus` takes the foreground by itself.
pub fn before_start() {
    #[cfg(target_os = "linux")]
    if asked_at_start() {
        linux::hand_token();
    }
}

/// A start or a second start asked for `ourstick:receive` (or macOS opened
/// it): Receive opens, unless a dialog is open, a receive is under way (the
/// window decides) or the app quits; the window comes to the front anyway.
/// Arrivals closer together than a few seconds are dropped.
pub fn arrived<R: Runtime>(app: &AppHandle<R>) {
    if !app.state::<Arrivals>().take(Instant::now()) {
        return;
    }
    if crate::dialog::quitting() {
        return;
    }
    let dialog_open = app
        .webview_windows()
        .keys()
        .any(|label| label.starts_with("confirm"));
    if !dialog_open {
        crate::receiving::opened::asked(app);
    }
}

/// macOS: the addresses opened with the app; only `ourstick:receive` counts.
#[cfg(target_os = "macos")]
pub fn opened(app: &AppHandle, urls: &[tauri::Url]) {
    if urls
        .iter()
        .any(|url| scheme::is_receive(std::ffi::OsStr::new(url.as_str())))
    {
        arrived(app);
        crate::tray::show_main(app);
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    //! The activation token, from a second start to the running app, over
    //! the session bus under a name of the app's own.

    use tauri::{AppHandle, Manager};

    const NAME: &str = "com.ourstick.OurStick.Activation";
    const PATH: &str = "/com/ourstick/OurStick/Activation";
    const INTERFACE: &str = "com.ourstick.OurStick.Activation";

    /// The running app's end: a token handed over is used by the next
    /// window brought forward, as a tray click's.
    struct Receiver(AppHandle);

    #[zbus::interface(name = "com.ourstick.OurStick.Activation")]
    impl Receiver {
        fn provide(&self, token: String) {
            self.0.state::<crate::windows::Activation>().provide(token);
        }
    }

    /// Kept for as long as the app runs.
    pub struct Served(#[allow(dead_code)] zbus::blocking::Connection);

    /// Serves the name, in `setup`. Not served (another app has it, no
    /// session bus): a second start's token is lost, and the system's
    /// "OurStick is ready" brings the window instead.
    pub fn serve(app: &AppHandle) {
        let served = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name(NAME))
            .and_then(|builder| builder.serve_at(PATH, Receiver(app.clone())))
            .and_then(|builder| builder.build());
        match served {
            Ok(connection) => {
                app.manage(Served(connection));
            }
            Err(error) => tracing::warn!(%error, "activation tokens cannot be handed over"),
        }
    }

    /// A second start's end: hands its token, if it has one, to the
    /// running app. Nothing runs: this start is the first, and GTK uses the
    /// token itself for its window.
    pub(super) fn hand_token() {
        let Some(token) = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|token| !token.is_empty())
        else {
            return;
        };
        let handed = zbus::blocking::Connection::session().and_then(|connection| {
            connection.call_method(Some(NAME), PATH, Some(INTERFACE), "Provide", &(token,))
        });
        if let Err(error) = handed {
            tracing::debug!(%error, "no running app took the activation token");
        }
    }
}
