//! The About window (ADR 0024 §10): what OurStick is, its version and
//! licences, and the links; opened from the tray, the sidebar's version
//! line and, on macOS, the app menu.

use anyhow::Result;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "about";

/// Opens the About window, or brings it forward.
pub fn open(app: &AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        crate::windows::bring_forward(&window);
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html#about".into()))
        .title("About OurStick")
        .focused(crate::windows::FOCUS_NEW)
        .inner_size(500.0, 720.0)
        .resizable(false)
        .build()?;
    crate::windows::show_new(&window);
    Ok(())
}

/// What About shows of the app itself.
#[derive(serde::Serialize)]
pub struct AboutView {
    version: String,
    /// The bundles' copyright line (`tauri.conf.json`, read by build.rs).
    copyright: &'static str,
}

#[tauri::command]
pub fn about_view(app: AppHandle) -> AboutView {
    AboutView {
        version: app.package_info().version.to_string(),
        copyright: env!("OURSTICK_COPYRIGHT"),
    }
}

/// Async: on Windows a window made from a synchronous command never loads
/// (WebView2 waits on the thread the command holds).
#[tauri::command]
pub async fn open_about(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|error| crate::words::told(&error))
}

/// A link of About, by its name: the page never passes a URL or a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    Website,
    Privacy,
    /// OurStick's own licence, installed beside it.
    Licence,
    /// The libraries' licences, installed beside it.
    Libraries,
    Report,
}

/// Where a link goes: an installed file, else (or for the others) a URL.
fn target(link: Link) -> (Option<&'static str>, &'static str) {
    match link {
        Link::Website => (None, "https://ourstick.com"),
        Link::Privacy => (None, "https://ourstick.com/privacy"),
        Link::Licence => (
            Some("LICENSE.txt"),
            "https://www.apache.org/licenses/LICENSE-2.0",
        ),
        Link::Libraries => (
            Some("THIRD-PARTY-LICENSES.html"),
            "https://ourstick.com/THIRD-PARTY-LICENSES",
        ),
        Link::Report => (None, "mailto:hi@ourstick.com"),
    }
}

#[tauri::command]
pub fn open_about_link(link: Link, app: AppHandle) -> Result<(), String> {
    use tauri::path::BaseDirectory;
    use tauri_plugin_opener::OpenerExt;
    let (file, url) = target(link);
    let installed = file
        .and_then(|file| app.path().resolve(file, BaseDirectory::Resource).ok())
        .filter(|path| path.is_file());
    let opened = match installed {
        Some(path) => app.opener().open_path(path.to_string_lossy(), None::<&str>),
        None => app.opener().open_url(url, None::<&str>),
    };
    opened.map_err(|error| format!("cannot open it: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_link_has_a_fixed_target() {
        assert_eq!(target(Link::Website).1, "https://ourstick.com");
        assert_eq!(target(Link::Report).1, "mailto:hi@ourstick.com");
        assert_eq!(target(Link::Libraries).0, Some("THIRD-PARTY-LICENSES.html"));
        assert_eq!(target(Link::Licence).0, Some("LICENSE.txt"));
        for link in [Link::Website, Link::Privacy, Link::Licence, Link::Libraries] {
            assert!(target(link).1.starts_with("https://"), "{link:?}");
        }
    }
}
