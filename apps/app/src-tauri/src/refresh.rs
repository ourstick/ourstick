//! What the tray and the window show of the app's sticks, kept as they are
//! now: the green dot, the tray's list, and drives ejected or mounted
//! behind the app's back.

use std::collections::HashMap;

use ourstick_core::received::display_name;
use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::sharing::*;
use crate::state::AppState;

/// The tray's green dot and the dialogs: anyone downloading from any stick.
pub async fn refresh_active(app: &AppHandle) {
    let state = app.state::<AppState>();
    let any = readers_now(&*state.sticks.lock().await) > 0;
    crate::tray::set_active(app, any);
}

/// The tray lists what is sent and received as it goes now, and its green
/// dot shows whether anyone gets something: the one place that tells it,
/// after anything that changes a thing's name or how it goes. One refresh
/// at a time, so an older one never shows over a newer one.
pub async fn refresh_tray(app: &AppHandle) {
    use crate::tray::{words, Shows, TrayThing};
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE_AT_A_TIME.lock().await;
    let state = app.state::<AppState>();
    let sticks = match state.sticks_now().await {
        Ok(sticks) => sticks,
        Err(error) => {
            return tracing::warn!(error = %format!("{error:#}"), "the tray stays as it was")
        }
    };
    let sharing = state.sticks.lock().await;
    let mut things: Vec<TrayThing> = sticks
        .into_iter()
        .map(|stick| TrayThing {
            shows: Shows::Sending(stick.stick_id),
            words: words::sending(sharing.get(&stick.stick_id)),
            name: stick.name,
        })
        .collect();
    let active = readers_now(&sharing) > 0;
    drop(sharing);
    let store = state.store.clone();
    let received = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let joined: Vec<(StickId, String)> = store
            .joined_sticks()?
            .into_iter()
            .filter(|joined| joined.kind == ourstick_core::StickKind::App)
            .map(|joined| {
                let name = display_name(joined.name.as_deref(), &joined.stick_id);
                (joined.stick_id, name)
            })
            .collect();
        let waiting: Vec<StickId> = store
            .waiting_links()?
            .into_iter()
            .map(|link| link.stick_id)
            .collect();
        Ok((joined, waiting, crate::login::shown(store.as_ref())))
    })
    .await;
    let (joined, waiting, start_at_login) = match received {
        Ok(Ok(received)) => received,
        Ok(Err(error)) => {
            return tracing::warn!(error = %format!("{error:#}"), "the tray stays as it was")
        }
        Err(error) => return tracing::warn!(%error, "the tray stays as it was"),
    };
    let receivers = app.state::<crate::receiving::Receivers>();
    for (stick_id, name) in joined {
        things.push(TrayThing {
            shows: Shows::Received(stick_id),
            words: words::received(&receivers.shown(&stick_id), false),
            name,
        });
    }
    for stick_id in waiting {
        things.push(TrayThing {
            shows: Shows::Received(stick_id),
            words: words::received(&receivers.shown(&stick_id), true),
            name: display_name(None, &stick_id),
        });
    }
    crate::tray::set_sticks(app, things, start_at_login);
    crate::tray::set_active(app, active);
}

/// [`refresh_tray`] in a second, once for all the changes meanwhile: what
/// is received changes often (each file), the menu at most once a second.
pub fn refresh_tray_soon(app: &AppHandle) {
    static PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if PENDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
        refresh_tray(&app).await;
    });
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
