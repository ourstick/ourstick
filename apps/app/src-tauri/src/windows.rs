//! Showing the app's windows. On Linux (GNOME) a window that asks for focus
//! without a fresh user action in it, e.g. one opened from the tray, is not
//! raised: GNOME shows an "OurStick is ready" notification instead. So on
//! Linux windows are raised by being put on top for a moment; elsewhere
//! they are also focused.

use tauri::{Runtime, WebviewWindow};

/// Whether new windows ask for focus when they open.
pub const FOCUS_NEW: bool = !cfg!(target_os = "linux");

/// Shows `window` (again) in front of the others. On Linux it is put on top
/// for a moment instead of asking for focus: in front, without the "is
/// ready" notification; a click gives it focus.
pub fn bring_forward<R: Runtime>(window: &WebviewWindow<R>) {
    // On GTK, show() also asks to be presented, which GNOME answers with the
    // notification: only for a hidden window, which must be shown.
    if !window.is_visible().unwrap_or(false) {
        let _ = window.show();
    }
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    if FOCUS_NEW {
        let _ = window.set_focus();
    } else {
        let _ = window.set_always_on_top(true);
        let window = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = window.set_always_on_top(false);
        });
    }
}
