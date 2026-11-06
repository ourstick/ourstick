//! The link of something sent: the link is the key, and goes to the
//! clipboard when the user asks (`Copy link`), nowhere else; on Windows
//! and macOS with the marks that keep it out of the clipboard's history.

use anyhow::Result;
use tauri::{AppHandle, State};
#[cfg(not(any(target_os = "android", windows, target_os = "macos")))]
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::state::{stick_id as stick_id_of, AppState, Sharing};

/// The link of `stick_id`, if it is shared and has been published.
async fn link(state: &AppState, stick_id: &str) -> Result<Option<String>, String> {
    let stick_id = stick_id_of(stick_id)?;
    Ok(match state.sticks.lock().await.get(&stick_id) {
        Some(Sharing::On(serving)) => serving.link.clone(),
        _ => None,
    })
}

async fn required_link(state: &AppState, stick_id: &str) -> Result<String, String> {
    link(state, stick_id)
        .await?
        .ok_or_else(|| "the link is not ready yet".to_owned())
}

#[cfg(not(target_os = "android"))]
fn shown(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub async fn copy_link(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let link = required_link(&state, &stick_id).await?;
    // Android: marked sensitive, so its clipboard preview hides it (ADR 0029).
    #[cfg(target_os = "android")]
    return crate::phone::link_out(&app, "copyLink", link).map_err(|error| format!("{error:#}"));
    // Windows and macOS: marked to stay out of the clipboard's history
    // (and on Windows its cloud).
    #[cfg(any(windows, target_os = "macos"))]
    return {
        let _ = app;
        // A system program on macOS, a clipboard busy for a moment on
        // Windows: off the async threads.
        tauri::async_runtime::spawn_blocking(move || crate::unkept_clipboard::put_text(&link))
            .await
            .map_err(shown)?
            .map_err(shown)
    };
    #[cfg(not(any(target_os = "android", windows, target_os = "macos")))]
    app.clipboard().write_text(link).map_err(shown)
}

/// Whether this device goes on sending once OurStick is not in front: a
/// desktop does; an Android phone from Android 14 (ADR 0029).
#[tauri::command]
pub async fn sends_in_background(app: AppHandle) -> bool {
    #[cfg(target_os = "android")]
    return crate::phone::sends_in_background(&app).unwrap_or(false);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        cfg!(desktop)
    }
}

/// "Send the link" on a phone: the system's share sheet, the link as text
/// (ADR 0029). Never shown in the window, never logged.
#[tauri::command]
pub async fn share_link(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let link = required_link(&state, &stick_id).await?;
    #[cfg(target_os = "android")]
    return crate::phone::link_out(&app, "shareLink", link).map_err(|error| format!("{error:#}"));
    #[cfg(not(target_os = "android"))]
    {
        let _ = (link, app);
        Err("Only a phone shares a link this way.".to_owned())
    }
}
