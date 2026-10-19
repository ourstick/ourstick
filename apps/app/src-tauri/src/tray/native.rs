//! Tauri's tray, for macOS (and later Windows).

use anyhow::Result;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

use super::{act, menu_lines, Action, Line, TrayThing};

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
    crate::drive::no_window("reg")
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

/// A label as the menu shows it: on Windows `&` marks an access key, and
/// macOS drops it as one (muda), so a stick's own `&` is doubled.
fn label(text: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        text.replace('&', "&&")
    } else {
        text.to_owned()
    }
}

fn item(app: &AppHandle, text: &str, action: Action) -> Result<MenuItem<Wry>> {
    Ok(MenuItem::with_id(
        app,
        action.id(),
        label(text),
        true,
        None::<&str>,
    )?)
}

/// The menu for the app's sticks and the login check.
fn menu(app: &AppHandle, sticks: &[TrayThing], start_at_login: bool) -> Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    for line in menu_lines(sticks, start_at_login) {
        match line {
            Line::Item(text, action) => menu.append(&item(app, &text, action)?)?,
            Line::Check(text, checked, action) => menu.append(&CheckMenuItem::with_id(
                app,
                action.id(),
                label(&text),
                true,
                checked,
                None::<&str>,
            )?)?,
            Line::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
        }
    }
    Ok(menu)
}

pub fn create(app: &AppHandle) -> Result<()> {
    let (image, template) = icon(false)?;
    TrayIconBuilder::with_id(TRAY)
        .icon(image)
        .icon_as_template(template)
        .tooltip("OurStick")
        .menu(&menu(app, &[], false)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let Some(action) = Action::from_id(event.id.as_ref()) else {
                return;
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

/// The app's sticks, as the menu lists them, and the login check: the
/// menu is made anew.
pub fn set_sticks(app: &AppHandle, sticks: Vec<TrayThing>, start_at_login: bool) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    match menu(app, &sticks, start_at_login) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "the tray menu stays as it was")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_name_keeps_its_ampersands() {
        let shown = label("Maria & Co");
        assert_eq!(
            shown,
            if cfg!(any(windows, target_os = "macos")) {
                "Maria && Co"
            } else {
                "Maria & Co"
            }
        );
    }

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
