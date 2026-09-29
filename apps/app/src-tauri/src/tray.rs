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
        crate::windows::bring_forward(&window);
    }
}

pub fn create(app: &AppHandle) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OurStick", true, None::<&str>)?;
    let share = MenuItem::with_id(app, "share", "Share…", true, None::<&str>)?;
    let mount = MenuItem::with_id(app, "mount", "Eject", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop sharing…", true, None::<&str>)?;
    app.manage(SharingItem(stop.clone()));
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &share, &mount, &separator, &stop, &quit])?;
    app.manage(MountItem(mount));
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
                "mount" => {
                    toggle_mount(app);
                    Ok(())
                }
                // "Stop sharing…" asks first; "Start sharing" just starts.
                "stop" => toggle_sharing(app),
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

/// The tray's Eject / Mount item, to change its text.
struct MountItem(MenuItem<tauri::Wry>);

/// The tray item reads "Eject" while mounted and "Mount" after an eject.
pub fn set_mounted(app: &AppHandle, mounted: bool) {
    if let Some(item) = app.try_state::<MountItem>() {
        let _ = item.0.set_text(if mounted { "Eject" } else { "Mount" });
    }
}

fn toggle_mount(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::state::AppState>();
        let Ok(Some(stick)) = state.drive_stick() else {
            return;
        };
        if crate::drive::is_mounted(&stick.root_path) {
            if let Err(error) = crate::drive::eject(&stick.root_path) {
                tracing::warn!(error = %format!("{error:#}"), "eject failed");
                let _ = tauri::Emitter::emit(
                    &app,
                    "share-problem",
                    crate::serving::ShareProblem::Failed {
                        message: format!("{error:#}"),
                    },
                );
            }
            set_mounted(&app, false);
            let _ = tauri::Emitter::emit(&app, "mount-changed", ());
        } else {
            crate::state::plug_in_and_share(&app);
        }
    });
}

/// The tray's Stop sharing / Start sharing item, to change its text.
struct SharingItem(MenuItem<tauri::Wry>);

/// The item reads "Stop sharing…" while sharing and "Start sharing" when not.
pub fn set_sharing(app: &AppHandle, sharing: bool) {
    if let Some(item) = app.try_state::<SharingItem>() {
        let _ = item.0.set_text(if sharing {
            "Stop sharing…"
        } else {
            "Start sharing"
        });
    }
}

fn toggle_sharing(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::state::AppState>();
    // Never wait on the main thread: a busy lock means sharing is changing,
    // so ask as if it were on.
    let sharing = state.sharing.try_lock().map_or(true, |sharing| {
        matches!(
            *sharing,
            crate::state::Sharing::On(_) | crate::state::Sharing::Starting
        )
    });
    if sharing {
        dialog::open(app, dialog::Mode::StopSharing)
    } else {
        crate::state::start_sharing(app);
        Ok(())
    }
}
