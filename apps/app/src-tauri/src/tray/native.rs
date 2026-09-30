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

/// Windows: a dark glyph on a light taskbar, a light one on a dark taskbar
/// (Windows 11's default). The theme is read once, at start.
#[cfg(windows)]
fn icon(active: bool) -> Result<(Image<'static>, bool)> {
    static LIGHT_TASKBAR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let light = *LIGHT_TASKBAR.get_or_init(light_taskbar);
    let bytes: &[u8] = match (light, active) {
        (true, false) => include_bytes!("../../icons/tray-template.png"),
        (true, true) => include_bytes!("../../icons/tray-grey-active.png"),
        (false, false) => include_bytes!("../../icons/tray-light.png"),
        (false, true) => include_bytes!("../../icons/tray-light-active.png"),
    };
    Ok((Image::from_bytes(bytes)?, false))
}

/// Whether the taskbar is light (`SystemUsesLightTheme` = 1); dark if the
/// setting cannot be read.
#[cfg(windows)]
fn light_taskbar() -> bool {
    std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "SystemUsesLightTheme",
        ])
        .output()
        .is_ok_and(|output| uses_light_theme(&String::from_utf8_lossy(&output.stdout)))
}

/// Reads `reg query`'s answer: `SystemUsesLightTheme    REG_DWORD    0x1`.
#[cfg(any(windows, test))]
fn uses_light_theme(answer: &str) -> bool {
    answer
        .lines()
        .find(|line| line.trim_start().starts_with("SystemUsesLightTheme"))
        .and_then(|line| line.split_whitespace().last())
        .is_some_and(|value| value == "0x1")
}

#[cfg(not(any(target_os = "macos", windows)))]
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
            // Not in the menu handler itself: a window made there never
            // loads on Windows (WebView2 waits on the event loop's thread).
            let app = app.clone();
            tauri::async_runtime::spawn(async move { act(&app, action) });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taskbar_theme_is_read_from_reg_query() {
        let light = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    SystemUsesLightTheme    REG_DWORD    0x1\r\n";
        assert!(uses_light_theme(light));
        assert!(!uses_light_theme(&light.replace("0x1", "0x0")));
        assert!(!uses_light_theme(
            "ERROR: The system was unable to find the specified registry key"
        ));
    }
}
