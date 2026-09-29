//! Tauri's tray, for macOS (and later Windows).

use anyhow::Result;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use super::{act, mount_label, sharing_label, Action};

const TRAY: &str = "main";

/// macOS tints a template image itself, for light and dark menu bars. A
/// template loses its colours, so the icon with the green dot is not one;
/// its grey glyph reads on both.
#[cfg(target_os = "macos")]
fn icon(active: bool) -> Result<(Image<'static>, bool)> {
    Ok(if active {
        (
            Image::from_bytes(include_bytes!("../../icons/tray-grey-active.png"))?,
            false,
        )
    } else {
        (
            Image::from_bytes(include_bytes!("../../icons/tray-template.png"))?,
            true,
        )
    })
}

#[cfg(not(target_os = "macos"))]
fn icon(active: bool) -> Result<(Image<'static>, bool)> {
    Ok(if active {
        (
            Image::from_bytes(include_bytes!("../../icons/tray-light-active.png"))?,
            false,
        )
    } else {
        (
            Image::from_bytes(include_bytes!("../../icons/tray-light.png"))?,
            false,
        )
    })
}

pub fn create(app: &AppHandle) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OurStick", true, None::<&str>)?;
    let share = MenuItem::with_id(app, "share", "Share…", true, None::<&str>)?;
    let mount = MenuItem::with_id(app, "mount", mount_label(true), true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", sharing_label(true), true, None::<&str>)?;
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
            let action = match event.id.as_ref() {
                "open" => Action::Open,
                "share" => Action::Share,
                "mount" => Action::Mount,
                "stop" => Action::Sharing,
                "quit" => Action::Quit,
                _ => return,
            };
            act(app, action);
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

pub fn set_mounted(app: &AppHandle, mounted: bool) {
    if let Some(item) = app.try_state::<MountItem>() {
        let _ = item.0.set_text(mount_label(mounted));
    }
}

/// The tray's Stop sharing / Start sharing item, to change its text.
struct SharingItem(MenuItem<tauri::Wry>);

pub fn set_sharing(app: &AppHandle, sharing: bool) {
    if let Some(item) = app.try_state::<SharingItem>() {
        let _ = item.0.set_text(sharing_label(sharing));
    }
}
