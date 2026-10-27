//! A sharing link opened from another app (ADR 0027, decision 6): on
//! Android it arrives at the app's link activity, which hands it over through
//! a plugin of the app's own (`LinkPlugin`). It is kept here until the
//! Receive view takes it, once; as every link, never logged nor put in a
//! message.

use std::sync::Mutex;

use tauri::State;
#[cfg(target_os = "android")]
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The link opened last and not yet taken.
#[derive(Default)]
pub struct OpenedLink(Mutex<Option<String>>);

impl OpenedLink {
    /// Keeps `text` if it is a whole sharing link. Anything else is dropped:
    /// Receive then opens as if Receive was pressed.
    #[cfg(any(target_os = "android", test))]
    fn arrived(&self, text: &str) {
        let link = super::clipboard::link_in(text);
        if link.is_none() {
            tracing::warn!("an opened link is not a whole sharing link");
        }
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = link;
    }

    fn take(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn waiting(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

/// A link arrived: kept, and the window shows Receive ("link-opened").
#[cfg(target_os = "android")]
fn arrived<R: Runtime>(app: &AppHandle<R>, text: &str) {
    app.state::<OpenedLink>().arrived(text);
    if let Err(error) = app.emit("link-opened", ()) {
        tracing::warn!(%error, "the window was not told of an opened link");
    }
}

/// Whether a link opened before the window was listening waits.
#[tauri::command]
pub fn link_opened(opened: State<'_, OpenedLink>) -> bool {
    opened.waiting()
}

/// The opened link, given once.
#[tauri::command]
pub fn take_opened_link(opened: State<'_, OpenedLink>) -> Option<String> {
    opened.take()
}

/// The plugin that receives the links from the link activity.
#[cfg(target_os = "android")]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri::ipc::Channel;

    #[derive(serde::Serialize)]
    struct Handler {
        handler: Channel,
    }

    #[derive(serde::Deserialize)]
    struct Event {
        link: String,
    }

    tauri::plugin::Builder::new("opened-link")
        .setup(|app, api| {
            let handle = api.register_android_plugin("com.ourstick.OurStick", "LinkPlugin")?;
            let app = app.clone();
            handle.run_mobile_plugin::<()>(
                "setHandler",
                Handler {
                    handler: Channel::new(move |body| {
                        let link = body.deserialize::<Event>().map(|event| event.link);
                        arrived(&app, &link.unwrap_or_default());
                        Ok(())
                    }),
                },
            )?;
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_whole_link_is_kept_and_given_once() {
        let opened = OpenedLink::default();
        opened.arrived("https://ourstick.com/s/#not-a-link");
        assert!(!opened.waiting());
        assert_eq!(opened.take(), None);

        let link = crate::receiving::clipboard::tests::a_link();
        opened.arrived(&link);
        assert!(opened.waiting());
        assert_eq!(opened.take(), Some(link));
        assert!(!opened.waiting());
        assert_eq!(opened.take(), None);
    }
}
