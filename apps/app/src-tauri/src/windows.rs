//! Showing the app's windows. On Linux (GNOME) a window that asks for focus
//! without a fresh user action in it, e.g. one opened from the tray, is not
//! raised: GNOME shows an "OurStick is ready" notification instead. So on
//! Linux windows are only shown; elsewhere they are also brought forward.

use tauri::{Runtime, WebviewWindow};

/// Whether new windows ask for focus when they open.
pub const FOCUS_NEW: bool = !cfg!(target_os = "linux");

/// Shows `window` (again), in front where the system allows it.
pub fn bring_forward<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.unminimize();
    if FOCUS_NEW {
        let _ = window.set_focus();
    }
}
