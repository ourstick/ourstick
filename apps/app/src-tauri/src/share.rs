//! The Share popup (docs/gui-spec.md, panel 2): a small window with the link,
//! its QR code, and ways to pass it on. The link is the key: it goes to this
//! window or the clipboard when the user asks, and nowhere else.

use anyhow::Result;
use tauri::image::Image;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::qr;
use crate::state::{stick_hex, stick_id as stick_id_of, AppState, Sharing};
use ourstick_core::StickId;

/// Pixels per module of the QR image put on the clipboard.
const QR_IMAGE_SCALE: usize = 8;

/// Shows the Share window of `stick_id`, making it if needed (one per
/// stick, so two sticks' links are never confused).
/// From the tray's event loop: the stick's name is a quick read there.
pub fn open(app: &AppHandle, stick_id: &StickId) -> Result<()> {
    if bring_forward(app, stick_id) {
        return Ok(());
    }
    let name = app.state::<AppState>().app_stick(stick_id)?.name;
    open_named(app, stick_id, &name)
}

fn window_label(stick_id: &StickId) -> String {
    format!("share-{}", stick_hex(stick_id))
}

/// Brings the Share window of `stick_id` forward, if there is one.
fn bring_forward(app: &AppHandle, stick_id: &StickId) -> bool {
    match app.get_webview_window(&window_label(stick_id)) {
        Some(window) => {
            crate::windows::bring_forward(&window);
            true
        }
        None => false,
    }
}

/// Closes the Share window of `stick_id`, if there is one: after a new link
/// or a removal it would wait for a link that never comes.
pub fn close(app: &AppHandle, stick_id: &StickId) {
    if let Some(window) = app.get_webview_window(&window_label(stick_id)) {
        let _ = window.close();
    }
}

fn open_named(app: &AppHandle, stick_id: &StickId, name: &str) -> Result<()> {
    let hex = stick_hex(stick_id);
    let label = window_label(stick_id);
    let page = format!("index.html#share/{hex}");
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(page.into()))
        .title(format!("Share \u{201c}{name}\u{201d}"))
        .focused(crate::windows::FOCUS_NEW)
        .inner_size(400.0, 260.0)
        .resizable(false)
        .build()?;
    crate::windows::show_new(&window);
    Ok(())
}

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

/// Async: on Windows a window made from a synchronous command never loads
/// (WebView2 waits on the thread the command holds).
#[tauri::command]
pub async fn open_share(stick_id: String, app: AppHandle) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    if bring_forward(&app, &id) {
        return Ok(());
    }
    let shown = |error: anyhow::Error| crate::words::told(&error);
    let name = app
        .state::<AppState>()
        .stick_now(&id)
        .await
        .map_err(shown)?
        .name;
    open_named(&app, &id, &name).map_err(shown)
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
