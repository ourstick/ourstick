//! Telling of a new version (ADR 0033): a look once a day in the
//! background, the line Home's foot and the tray show, About's setting, and
//! the window's told-once notes. The core decides all of it; this only reads
//! the database off the async runtime, shows, and opens the fixed download
//! page.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::new_version::{
    is_due, keep_outcome, notice_line, read_outcome, release_day, security_to_tell, set_looking,
    this_version, told_looking, told_security, HttpSource, LookFailure, NoticeKey, NoticeKeys,
    NoticeLine, NoticeStore, System, NOTICE_URL,
};
use ourstick_core::store::SqliteStore;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::blocking;

/// How often the loop wakes to see whether a look is due.
const WAKE: Duration = Duration::from_secs(10 * 60);
/// The page `Get it` opens: the site's downloads, which pick this system.
const DOWNLOAD_PAGE: &str = "https://ourstick.com/#download";

/// The app's own connection to the database, and a nudge for the loop.
pub struct NoticeState {
    store: Arc<SqliteStore>,
    /// Some when this build reads the notice (rule 13).
    looks: bool,
    wake: Arc<tokio::sync::Notify>,
}

/// Starts the looks, on a desktop (rule 16), unless this build reads no
/// notice: a debug build without a test file, or one with its own data
/// folder (rule 13).
pub fn start(app: &AppHandle, data_dir: &Path) {
    let store = match SqliteStore::open(data_dir.join("state.sqlite")) {
        Ok(store) => Arc::new(store),
        Err(error) => return tracing::warn!(%error, "no looks for a new version"),
    };
    let wake = Arc::new(tokio::sync::Notify::new());
    let reading = System::this().and_then(|system| Some((system, reading()?)));
    app.manage(NoticeState {
        store: store.clone(),
        looks: reading.is_some(),
        wake: wake.clone(),
    });
    let Some((system, (source, keys))) = reading else {
        return tracing::info!("this build does not look for new versions");
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let due_store = store.clone();
            let due =
                blocking(move || Ok(is_due(&due_store.version_look()?, SystemClock.now_ms())))
                    .await
                    .unwrap_or(false);
            if due {
                let outcome = read_outcome(&source, &keys, system).await;
                let keep_store = store.clone();
                match blocking(move || {
                    Ok(keep_outcome(
                        keep_store.as_ref(),
                        outcome,
                        SystemClock.now_ms(),
                    )?)
                })
                .await
                {
                    Ok(true) => changed(&app).await,
                    Ok(false) => {}
                    Err(error) => tracing::warn!(%error, "a look for a new version was not kept"),
                }
            }
            let _ = tokio::time::timeout(WAKE, wake.notified()).await;
        }
    });
}

/// Where this build reads the notice, and with which keys.
fn reading() -> Option<(HttpSource, NoticeKeys)> {
    let ourstick = NoticeKeys::ourstick();
    if cfg!(debug_assertions) {
        // A test file and a test release key, or nothing.
        let url = std::env::var("OURSTICK_VERSION_URL").ok()?;
        let key = NoticeKey::parse(&std::env::var("OURSTICK_VERSION_KEY").ok()?)?;
        let keys = NoticeKeys {
            release: key,
            recovery: ourstick.recovery,
        };
        return HttpSource::new(&url).ok().map(|source| (source, keys));
    }
    if std::env::var_os("OURSTICK_DATA_DIR").is_some() {
        return None;
    }
    HttpSource::new(NOTICE_URL)
        .inspect_err(|error| tracing::warn!(%error, "no looks for a new version"))
        .ok()
        .map(|source| (source, ourstick))
}

/// Something changed: the window and the tray show it again, and a
/// security line not yet told brings the window up (rule 8, as a due thing
/// does, ADR 0024 rule 3).
async fn changed(app: &AppHandle) {
    let _ = app.emit("version-notice-changed", ());
    crate::refresh::refresh_tray(app).await;
    if let Ok(view) = view(app).await {
        if view.tell_security {
            crate::tray::show_main(app);
        }
    }
}

/// One line, as Home's foot shows it: the words, and the press.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineView {
    /// "newer", "security", "key-changed" or "old".
    kind: &'static str,
    text: String,
    action: &'static str,
}

/// The words of a line; none for nothing.
pub fn line_view(line: &NoticeLine) -> Option<LineView> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    Some(match line {
        NoticeLine::Nothing => return None,
        NoticeLine::Newer { version } => LineView {
            kind: "newer",
            text: format!("OurStick {version} is out"),
            action: "Get it",
        },
        NoticeLine::Security { version } => LineView {
            kind: "security",
            text: format!("{version} fixes a security problem in this version"),
            action: "Get it",
        },
        NoticeLine::ReleaseKeyChanged => LineView {
            kind: "key-changed",
            text: "OurStick's signing key changed: install the new version by hand".to_owned(),
            action: "Get it",
        },
        NoticeLine::Old { year, month } => LineView {
            kind: "old",
            text: format!(
                "This version is from {} {year}. A newer one may fix security problems",
                MONTHS[(*month as usize).clamp(1, 12) - 1]
            ),
            action: "ourstick.com",
        },
    })
}

/// What the window shows of it all.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeView {
    line: Option<LineView>,
    /// About shows the setting (a desktop build that reads the notice).
    setting: bool,
    looking: bool,
    last_good_ms: Option<u64>,
    failing_since_ms: Option<u64>,
    /// "no-connection", "refused" or "not-the-file".
    failure: Option<&'static str>,
    /// Told once: the app now looks (rule 11).
    tell_looking: bool,
    /// Told once: the security line (rule 8).
    tell_security: bool,
}

async fn view(app: &AppHandle) -> Result<NoticeView, String> {
    let state = app.try_state::<NoticeState>().ok_or("not started")?;
    let (store, looks) = (state.store.clone(), state.looks);
    blocking(move || {
        let look = store.version_look()?;
        let release_key = NoticeKeys::ourstick().release.id_hex();
        let line = notice_line(
            &look,
            &this_version(),
            &release_key,
            release_day(),
            SystemClock.now_ms(),
        );
        Ok(NoticeView {
            tell_security: looks && security_to_tell(&look, &line),
            line: line_view(&line).filter(|_| looks),
            setting: looks,
            looking: look.looking,
            last_good_ms: look.last_good_ms,
            failing_since_ms: look.failing_since_ms,
            failure: look.failure.map(LookFailure::as_str),
            tell_looking: looks && look.looking && !look.looking_told,
        })
    })
    .await
}

/// At start, before the window shows: a security line not yet told makes
/// it show (rule 8), also when started at login. Blocking, as the setup is.
pub fn security_to_tell_now(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<NoticeState>().filter(|state| state.looks) else {
        return false;
    };
    let release_key = NoticeKeys::ourstick().release.id_hex();
    state.store.version_look().is_ok_and(|look| {
        let line = notice_line(
            &look,
            &this_version(),
            &release_key,
            release_day(),
            SystemClock.now_ms(),
        );
        security_to_tell(&look, &line)
    })
}

/// The tray's line above About: a newer version, a security fix or a
/// changed key, never the old-version line (Home's foot only, rule 10a).
pub async fn tray_line(app: &AppHandle) -> Option<String> {
    let line = view(app).await.ok()?.line?;
    (line.kind != "old").then(|| format!("{} \u{b7} {}", line.text, line.action))
}

#[tauri::command]
pub async fn version_notice(app: AppHandle) -> Result<NoticeView, String> {
    view(&app).await
}

/// "Look for new versions" on or off; on looks at once.
#[tauri::command]
pub async fn set_look_for_versions(
    on: bool,
    app: AppHandle,
    state: State<'_, NoticeState>,
) -> Result<(), String> {
    let store = state.store.clone();
    blocking(move || Ok(set_looking(store.as_ref(), on)?)).await?;
    state.wake.notify_one();
    changed(&app).await;
    Ok(())
}

/// A told-once note was read.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Told {
    Looking,
    Security,
}

#[tauri::command]
pub async fn version_told(what: Told, state: State<'_, NoticeState>) -> Result<(), String> {
    let store = state.store.clone();
    blocking(move || {
        match what {
            Told::Looking => told_looking(store.as_ref())?,
            Told::Security => told_security(store.as_ref())?,
        }
        Ok(())
    })
    .await
}

/// `Get it`: the fixed download page, never a URL from the notice.
#[tauri::command]
pub fn open_download_page(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(DOWNLOAD_PAGE, None::<&str>)
        .map_err(|error| format!("cannot open it: {error}"))
}

/// From the tray.
pub fn open_download_page_from_tray(app: &AppHandle) {
    if let Err(error) = open_download_page(app.clone()) {
        tracing::warn!(%error, "the download page did not open");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_has_its_words() {
        assert_eq!(line_view(&NoticeLine::Nothing), None);
        let newer = line_view(&NoticeLine::Newer {
            version: "0.3.0".into(),
        })
        .unwrap();
        assert_eq!(newer.text, "OurStick 0.3.0 is out");
        let security = line_view(&NoticeLine::Security {
            version: "0.3.0".into(),
        })
        .unwrap();
        assert_eq!(security.kind, "security");
        assert_eq!(
            security.text,
            "0.3.0 fixes a security problem in this version"
        );
        let old = line_view(&NoticeLine::Old {
            year: 2027,
            month: 3,
        })
        .unwrap();
        assert_eq!(
            old.text,
            "This version is from March 2027. A newer one may fix security problems"
        );
        assert_eq!(old.action, "ourstick.com");
    }

    #[test]
    fn the_download_page_is_fixed_and_on_the_site() {
        assert!(DOWNLOAD_PAGE.starts_with("https://ourstick.com/"));
    }
}
