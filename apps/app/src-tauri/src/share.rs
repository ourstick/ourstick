//! The link of something sent, and its QR code: the link is the key, and
//! goes to the clipboard when the user asks (`Copy link`), nowhere else.

use anyhow::Result;
use tauri::image::Image;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::qr;
use crate::state::{stick_id as stick_id_of, AppState, Sharing};

/// Pixels per module of the QR image put on the clipboard.
const QR_IMAGE_SCALE: usize = 8;

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

fn shown(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The link and its QR code as SVG, or nothing while there is no link yet.
#[derive(Debug, serde::Serialize)]
pub struct ShareView {
    link: String,
    qr_svg: String,
}

#[tauri::command]
pub async fn share_view(
    stick_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ShareView>, String> {
    let Some(link) = link(&state, &stick_id).await? else {
        return Ok(None);
    };
    let qr_svg = qr::svg(&link).map_err(shown)?;
    Ok(Some(ShareView { link, qr_svg }))
}

#[tauri::command]
pub async fn copy_link(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let link = required_link(&state, &stick_id).await?;
    app.clipboard().write_text(link).map_err(shown)
}

#[tauri::command]
pub async fn copy_qr_image(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let link = required_link(&state, &stick_id).await?;
    let (pixels, width, height) = qr::rgba(&link, QR_IMAGE_SCALE).map_err(shown)?;
    app.clipboard()
        .write_image(&Image::new_owned(pixels, width, height))
        .map_err(shown)
}
