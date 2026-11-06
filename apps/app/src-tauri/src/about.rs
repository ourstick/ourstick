//! The About window (ADR 0024 §10): what OurStick is, its version and
//! licences, and the links; opened from the tray, the sidebar's version
//! line and, on macOS, the app menu.

use anyhow::Result;
use tauri::{AppHandle, Manager};

const LABEL: &str = "about";

/// Opens the About window, or brings it forward.
pub fn open(app: &AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        crate::windows::bring_forward(&window);
        return Ok(());
    }
    let window = crate::windows::own_window(
        app,
        crate::windows::OwnWindow {
            label: LABEL,
            page: "index.html#about".to_owned(),
            title: "About OurStick",
            size: SIZE,
            dialog: false,
            resizable: false,
        },
    )?;
    crate::windows::show_new(&window);
    Ok(())
}

/// About's size: all of it without scrolling (Linux's font is the largest),
/// also with the warning under "Look for new versions" when it is off.
const SIZE: (f64, f64) = (520.0, 800.0);

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
    /// OurStick's own licence.
    Licence,
    /// The libraries' licences.
    Libraries,
    Report,
}

/// Where a link goes.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// Opened by the system: the browser, the mail app.
    Url(&'static str),
    /// A page of the app's own (made by vite.config.ts), in a window of its
    /// own: the licence the app was built with, readable on every system.
    /// The system's default app may be unable to read the copy installed
    /// beside OurStick: on Ubuntu the Firefox snap cannot read /usr/lib.
    Page {
        label: &'static str,
        page: &'static str,
        title: &'static str,
    },
}

fn target(link: Link) -> Target {
    match link {
        Link::Website => Target::Url("https://ourstick.com"),
        Link::Privacy => Target::Url("https://ourstick.com/privacy"),
        Link::Licence => Target::Page {
            label: "licence",
            page: "LICENSE.html",
            title: "OurStick's licence",
        },
        Link::Libraries => Target::Page {
            label: "libraries",
            page: "THIRD-PARTY-LICENSES.html",
            title: "Licences of the libraries in OurStick",
        },
        Link::Report => Target::Url("mailto:hi@ourstick.com"),
    }
}

/// Async: a window made from a synchronous command never loads on Windows
/// (see [`open_about`]).
#[tauri::command]
pub async fn open_about_link(link: Link, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    match target(link) {
        Target::Url(url) => app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|error| format!("cannot open it: {error}")),
        Target::Page { label, page, title } => {
            open_page(&app, label, page, title).map_err(|error| crate::words::told(&error))
        }
    }
}

/// Shows `page` in its window, or brings that window forward.
fn open_page(app: &AppHandle, label: &str, page: &str, title: &str) -> Result<()> {
    if let Some(window) = app.get_webview_window(label) {
        crate::windows::bring_forward(&window);
        return Ok(());
    }
    let window = crate::windows::own_window(
        app,
        crate::windows::OwnWindow {
            label,
            page: page.to_owned(),
            title,
            size: (720.0, 680.0),
            dialog: false,
            resizable: true,
        },
    )?;
    crate::windows::show_new(&window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_link_has_a_fixed_target() {
        assert_eq!(target(Link::Website), Target::Url("https://ourstick.com"));
        assert_eq!(
            target(Link::Privacy),
            Target::Url("https://ourstick.com/privacy")
        );
        assert_eq!(target(Link::Report), Target::Url("mailto:hi@ourstick.com"));
        let Target::Page { page, label, .. } = target(Link::Licence) else {
            panic!("the licence is a page of the app");
        };
        assert_eq!((label, page), ("licence", "LICENSE.html"));
        let Target::Page { page, label, .. } = target(Link::Libraries) else {
            panic!("the libraries' licences are a page of the app");
        };
        assert_eq!((label, page), ("libraries", "THIRD-PARTY-LICENSES.html"));
    }

    /// The licence windows' pages are the ones the build makes.
    #[test]
    fn the_build_makes_the_licence_pages() {
        let vite = include_str!("../../vite.config.ts");
        for link in [Link::Licence, Link::Libraries] {
            let Target::Page { page, .. } = target(link) else {
                panic!("{link:?} is a page of the app");
            };
            assert!(vite.contains(&format!("\"{page}\": () =>")), "{page}");
        }
    }

    /// No other window takes a licence window's label: it would be brought
    /// forward instead of the licence.
    #[test]
    fn the_licence_windows_have_labels_of_their_own() {
        for link in [Link::Licence, Link::Libraries] {
            let Target::Page { label, .. } = target(link) else {
                panic!("{link:?} is a page of the app");
            };
            assert_ne!(label, LABEL);
            assert_ne!(label, "main");
            assert!(!label.starts_with("confirm") && !label.starts_with("share"));
        }
    }
}
