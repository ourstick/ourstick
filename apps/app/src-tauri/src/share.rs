//! The Share popup (docs/gui-spec.md, panel 2): a small window with the link,
//! its QR code, and ways to pass it on. The link is the key: it goes to this
//! window, the clipboard or the user's mail program when the user asks, and
//! nowhere else.

use anyhow::{Context, Result};
use tauri::image::Image;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::qr;
use crate::state::{AppState, Sharing};

/// Pixels per module of the QR image put on the clipboard.
const QR_IMAGE_SCALE: usize = 8;

/// Shows the Share window, making it if needed.
pub fn open(app: &AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window("share") {
        crate::windows::bring_forward(&window);
        return Ok(());
    }
    let window =
        WebviewWindowBuilder::new(app, "share", WebviewUrl::App("index.html#share".into()))
            .title("Share OurStick")
            .focused(crate::windows::FOCUS_NEW)
            .inner_size(400.0, 560.0)
            .resizable(false)
            .build()?;
    crate::windows::show_new(&window);
    Ok(())
}

/// The link, if the stick is shared and has been published.
async fn link(state: &AppState) -> Option<String> {
    match &*state.sharing.lock().await {
        Sharing::On(serving) => serving.link.clone(),
        _ => None,
    }
}

async fn required_link(state: &AppState) -> Result<String, String> {
    link(state)
        .await
        .ok_or_else(|| "the link is not ready yet".to_owned())
}

fn shown(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Async: on Windows a window made from a synchronous command never loads
/// (WebView2 waits on the thread the command holds).
#[tauri::command]
pub async fn open_share(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|error| format!("{error:#}"))
}

/// The link and its QR code as SVG, or nothing while there is no link yet.
#[derive(Debug, serde::Serialize)]
pub struct ShareView {
    link: String,
    qr_svg: String,
}

#[tauri::command]
pub async fn share_view(state: State<'_, AppState>) -> Result<Option<ShareView>, String> {
    let Some(link) = link(&state).await else {
        return Ok(None);
    };
    let qr_svg = qr::svg(&link).map_err(shown)?;
    Ok(Some(ShareView { link, qr_svg }))
}

#[tauri::command]
pub async fn copy_link(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let link = required_link(&state).await?;
    app.clipboard().write_text(link).map_err(shown)
}

#[tauri::command]
pub async fn copy_qr_image(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let link = required_link(&state).await?;
    let (pixels, width, height) = qr::rgba(&link, QR_IMAGE_SCALE).map_err(shown)?;
    app.clipboard()
        .write_image(&Image::new_owned(pixels, width, height))
        .map_err(shown)
}

/// Opens the user's mail program with the link in the body.
#[tauri::command]
pub async fn send_by_email(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let link = required_link(&state).await?;
    let url = mailto(&link);
    app.opener()
        .open_url(url, None::<&str>)
        .context("no mail program to open")
        .map_err(|error| format!("{error:#}"))
}

/// A `mailto:` URL with a subject and the link as the body.
fn mailto(link: &str) -> String {
    let body = format!("Here is my OurStick. Open the link to see the files:\n\n{link}\n");
    format!(
        "mailto:?subject={}&body={}",
        percent_encode("A stick for you"),
        percent_encode(&body)
    )
}

/// Percent-encodes everything but the unreserved characters (RFC 3986).
fn percent_encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mail_carries_the_whole_link_encoded() {
        let link = "https://ourstick.com/s/ABC#secret";
        let url = mailto(link);
        assert!(url.starts_with("mailto:?subject=A%20stick%20for%20you&body="));
        // '#', '/', ':' and newlines must not end or split the URL.
        assert!(url.contains("https%3A%2F%2Fourstick.com%2Fs%2FABC%23secret"));
        assert!(!url[7..].contains(['#', '/', '\n', ' ']));
    }
}
