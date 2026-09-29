//! The tray icon (docs/gui-spec.md, panel 3): always there while the app
//! runs, with a green dot while someone downloads, and the app's menu.

use anyhow::Result;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::{dialog, share};

const TRAY: &str = "main";

/// macOS tints a template image itself, for light and dark menu bars. A
/// template loses its colours, so the icon with the green dot is not one;
/// its grey glyph reads on both.
#[cfg(target_os = "macos")]
fn icon(active: bool) -> Result<(Image<'static>, bool)> {
    Ok(if active {
        (
            Image::from_bytes(include_bytes!("../icons/tray-grey-active.png"))?,
            false,
        )
    } else {
        (
            Image::from_bytes(include_bytes!("../icons/tray-template.png"))?,
            true,
        )
    })
}

/// Linux panels are dark by default (GNOME's top bar): a light glyph.
#[cfg(not(target_os = "macos"))]
fn icon(active: bool) -> Result<(Image<'static>, bool)> {
    Ok(if active {
        (
            Image::from_bytes(include_bytes!("../icons/tray-light-active.png"))?,
            false,
        )
    } else {
        (
            Image::from_bytes(include_bytes!("../icons/tray-light.png"))?,
            false,
        )
    })
}

/// Shows the main window again (from the tray, or a second launch).
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn create(app: &AppHandle) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OurStick", true, None::<&str>)?;
    let share = MenuItem::with_id(app, "share", "Share…", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop sharing…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &share, &separator, &stop, &quit])?;
    let (image, template) = icon(false)?;
    TrayIconBuilder::with_id(TRAY)
        .icon(image)
        .icon_as_template(template)
        .tooltip("OurStick")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "open" => {
                    show_main(app);
                    Ok(())
                }
                "share" => share::open(app),
                "stop" => dialog::open(app, dialog::Mode::StopSharing),
                "quit" => dialog::open(app, dialog::Mode::Quit),
                _ => Ok(()),
            };
            if let Err(error) = result {
                tracing::warn!(error = %format!("{error:#}"), "tray menu action failed");
            }
        })
        .build(app)?;
    Ok(())
}

/// The green dot, while someone downloads.
pub fn set_active(app: &AppHandle, active: bool) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    if let Ok((image, template)) = icon(active) {
        let _ = tray.set_icon(Some(image));
        let _ = tray.set_icon_as_template(template);
    }
}
