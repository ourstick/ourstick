//! What the app needs from Android that Rust cannot reach, through the
//! app's own plugin (`PhonePlugin.kt`, ADR 0027): the sharing links opened
//! from other apps, the screen kept on while a download runs, and a
//! received folder opened in Files.

use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime};

/// The plugin, and how many hold the screen on.
struct Phone<R: Runtime> {
    plugin: PluginHandle<R>,
    screen_holders: Mutex<usize>,
}

/// The plugin, registered at start.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    #[derive(serde::Serialize)]
    struct Handler {
        handler: Channel,
    }

    #[derive(serde::Deserialize)]
    struct Event {
        link: String,
    }

    tauri::plugin::Builder::new("phone")
        .setup(|app, api| {
            let plugin = api.register_android_plugin("com.ourstick.OurStick", "PhonePlugin")?;
            let opened = app.clone();
            plugin.run_mobile_plugin::<()>(
                "setHandler",
                Handler {
                    handler: Channel::new(move |body| {
                        let link = body.deserialize::<Event>().map(|event| event.link);
                        crate::receiving::opened::arrived(&opened, &link.unwrap_or_default());
                        Ok(())
                    }),
                },
            )?;
            app.manage(Phone {
                plugin,
                screen_holders: Mutex::new(0),
            });
            Ok(())
        })
        .build()
}

/// While held (by any number at once), the screen stays on: Android stops
/// a download once the screen is off and the phone locked.
pub struct ScreenOn<R: Runtime>(AppHandle<R>);

impl<R: Runtime> ScreenOn<R> {
    pub fn hold(app: &AppHandle<R>) -> anyhow::Result<Self> {
        screen_holders(app, true)?;
        Ok(Self(app.clone()))
    }
}

impl<R: Runtime> Drop for ScreenOn<R> {
    fn drop(&mut self) {
        if let Err(error) = screen_holders(&self.0, false) {
            tracing::warn!(error = %format!("{error:#}"), "the screen was not let go");
        }
    }
}

/// One holder more (`more`) or less; the screen is kept on from the first
/// and let go after the last.
fn screen_holders<R: Runtime>(app: &AppHandle<R>, more: bool) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct KeepScreenOn {
        on: bool,
    }

    let phone = phone(app)?;
    let mut holders = phone
        .screen_holders
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = *holders;
    *holders = if more {
        before + 1
    } else {
        before.saturating_sub(1)
    };
    if (before == 0) != (*holders == 0) {
        phone
            .plugin
            .run_mobile_plugin::<()>("keepScreenOn", KeepScreenOn { on: more })?;
    }
    Ok(())
}

/// Opens `folder` (in `Download/OurStick`) in Files.
pub fn open_folder<R: Runtime>(app: &AppHandle<R>, folder: &std::path::Path) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct OpenFolder {
        path: String,
    }

    phone(app)?.plugin.run_mobile_plugin::<()>(
        "openFolder",
        OpenFolder {
            path: folder.to_string_lossy().into_owned(),
        },
    )?;
    Ok(())
}

/// The system's picker for `pick`, waiting for the user: each chosen
/// place's content URI and name, kept readable (ADR 0029). Blocking.
pub fn choose<R: Runtime>(
    app: &AppHandle<R>,
    pick: crate::send::Pick,
) -> anyhow::Result<Vec<(String, String)>> {
    #[derive(serde::Serialize)]
    struct Choose {
        pick: crate::send::Pick,
    }

    #[derive(serde::Deserialize)]
    struct One {
        uri: String,
        name: String,
    }

    #[derive(serde::Deserialize)]
    struct Chosen {
        chosen: Vec<One>,
    }

    let chosen: Chosen = phone(app)?
        .plugin
        .run_mobile_plugin("choose", Choose { pick })?;
    Ok(chosen
        .chosen
        .into_iter()
        .map(|one| (one.uri, one.name))
        .collect())
}

/// The link out of the app through the plugin's `command` (`shareLink`,
/// `copyLink`); never logged.
pub fn link_out<R: Runtime>(app: &AppHandle<R>, command: &str, link: String) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct Link {
        link: String,
    }

    phone(app)?
        .plugin
        .run_mobile_plugin::<()>(command, Link { link })?;
    Ok(())
}

fn phone<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<tauri::State<'_, Phone<R>>> {
    app.try_state::<Phone<R>>()
        .ok_or_else(|| anyhow::anyhow!("the app's phone plugin did not start"))
}
