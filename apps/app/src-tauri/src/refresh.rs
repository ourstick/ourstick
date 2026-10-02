//! What the tray and the window show of the app's sticks, kept as they are
//! now: the green dot, the tray's list, and drives ejected or mounted
//! behind the app's back.

use std::collections::HashMap;

use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::sharing::*;
use crate::state::{AppState, Sharing};

/// The tray's green dot and the dialogs: anyone downloading from any stick.
pub async fn refresh_active(app: &AppHandle) {
    let state = app.state::<AppState>();
    let any = readers_now(&*state.sticks.lock().await) > 0;
    crate::tray::set_active(app, any);
}

/// The tray lists the app's sticks as they are now, and its green dot
/// shows whether anyone downloads: the one place that tells it, after
/// anything that changes a stick's name, drive or sharing. One refresh at a
/// time, so an older one never shows over a newer one.
pub async fn refresh_tray(app: &AppHandle) {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE_AT_A_TIME.lock().await;
    let state = app.state::<AppState>();
    let sticks = match state.sticks_now().await {
        Ok(sticks) => sticks,
        Err(error) => {
            return tracing::warn!(error = %format!("{error:#}"), "the tray stays as it was")
        }
    };
    let mounted = tauri::async_runtime::spawn_blocking(move || {
        sticks
            .into_iter()
            .map(|stick| (crate::drive::is_mounted(&stick), stick))
            .collect::<Vec<_>>()
    })
    .await;
    let mounted = match mounted {
        Ok(mounted) => mounted,
        Err(error) => return tracing::warn!(%error, "the tray stays as it was"),
    };
    let sharing = state.sticks.lock().await;
    let shown = mounted
        .into_iter()
        .map(|(mounted, stick)| crate::tray::TrayStick {
            id: stick.stick_id,
            sharing: sharing.get(&stick.stick_id).is_some_and(Sharing::is_on),
            doing: match sharing.get(&stick.stick_id) {
                Some(Sharing::Busy(doing)) => Some(*doing),
                _ => None,
            },
            name: stick.name,
            mounted,
        })
        .collect();
    let active = readers_now(&sharing) > 0;
    drop(sharing);
    crate::tray::set_sticks(app, shown);
    crate::tray::set_active(app, active);
}

/// Tells the UI (event `mount-changed`) when a stick is ejected or mounted
/// behind the app's back, e.g. with Finder's eject button, and finishes
/// what waited for a drive when one comes back. Every few seconds.
pub fn watch_mount(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<StickId, bool> = HashMap::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let Ok(sticks) = app.state::<AppState>().sticks_now().await else {
                continue;
            };
            let mut changed = false;
            for stick in sticks {
                let id = stick.stick_id;
                let mounted =
                    tauri::async_runtime::spawn_blocking(move || crate::drive::is_mounted(&stick))
                        .await
                        .unwrap_or(false);
                let before = last.insert(id, mounted);
                if before.is_some_and(|before| before != mounted) {
                    changed = true;
                    if mounted {
                        finish_pending_off_thread(&app).await;
                    }
                }
            }
            if changed {
                let _ = app.emit("mount-changed", ());
                refresh_tray(&app).await;
            }
        }
    });
}
