//! The macOS app menu. The default one quits at once on ⌘Q; ours asks first
//! (docs/gui-spec.md: Quit is always confirmed), and its About opens the
//! app's own window, as on the other systems (ADR 0024). Edit and Window
//! keep the usual items (copy and paste in the link field, closing windows).

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Wry};

/// The id of our Quit item (not "quit": the tray has one of those).
pub const QUIT: &str = "app-quit";
/// The id of our About item (the tray has an "about" too).
pub const ABOUT: &str = "app-about";

pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let quit = MenuItem::with_id(app, QUIT, "Quit OurStick", true, Some("CmdOrCtrl+Q"))?;
    let about = MenuItem::with_id(app, ABOUT, "About OurStick", true, None::<&str>)?;
    let app_menu = Submenu::with_items(
        app,
        "OurStick",
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Hide OurStick"))?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}
