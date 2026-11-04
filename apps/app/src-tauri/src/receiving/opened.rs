//! Receive opened from outside the window, kept here until the Receive view
//! takes it, once. On Android a sharing link opened from another app (ADR
//! 0027, decision 6), handed over by the app's own plugin (`crate::phone`).
//! On a desktop `ourstick:receive` (ADR 0030 §1), or a new link found when
//! the window came back (§4): Receive then reads the clipboard itself and
//! asks before looking (§2). As every link, never logged nor put in a
//! message.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// What Receive was opened with.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Opened {
    /// A whole sharing link: looked at at once. Only a phone gets one.
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    Link { link: String },
    /// Nothing: the link is on the clipboard, if anywhere, and is looked at
    /// only once the user says so.
    Asked,
}

/// Never the link: a debug print must not put a stick's key in a log.
impl std::fmt::Debug for Opened {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Link { .. } => formatter.write_str("Link { .. }"),
            Self::Asked => formatter.write_str("Asked"),
        }
    }
}

/// What opened Receive last and was not yet taken.
#[derive(Default)]
pub struct OpenedLink(Mutex<Option<Opened>>);

impl OpenedLink {
    /// Keeps `text` if it is a whole sharing link. Anything else is dropped:
    /// Receive then opens as if Receive was pressed.
    #[cfg(any(target_os = "android", test))]
    fn arrived(&self, text: &str) {
        let link = super::clipboard::link_in(text);
        if link.is_none() {
            tracing::warn!("an opened link is not a whole sharing link");
        }
        self.set(link.map(|link| Opened::Link { link }));
    }

    /// Receive is to open with nothing in it.
    #[cfg_attr(mobile, allow(dead_code))]
    pub(crate) fn asked(&self) {
        self.set(Some(Opened::Asked));
    }

    fn set(&self, opened: Option<Opened>) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = opened;
    }

    fn take(&self) -> Option<Opened> {
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
pub(crate) fn arrived<R: Runtime>(app: &AppHandle<R>, text: &str) {
    app.state::<OpenedLink>().arrived(text);
    tell(app);
}

/// The window is told to show Receive, which takes what waits here.
pub(crate) fn tell<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.emit("link-opened", ()) {
        tracing::warn!(%error, "the window was not told of an opened link");
    }
}

/// Whether what opened Receive before the window was listening waits.
#[tauri::command]
pub fn link_opened(opened: State<'_, OpenedLink>) -> bool {
    opened.waiting()
}

/// What opened Receive, given once.
#[tauri::command]
pub fn take_opened_link(opened: State<'_, OpenedLink>) -> Option<Opened> {
    opened.take()
}

/// Receive opened with nothing in it, as `ourstick:receive` asks.
#[cfg_attr(mobile, allow(dead_code))]
pub(crate) fn asked<R: Runtime>(app: &AppHandle<R>) {
    app.state::<OpenedLink>().asked();
    tell(app);
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
        assert_eq!(opened.take(), Some(Opened::Link { link }));
        assert!(!opened.waiting());
        assert_eq!(opened.take(), None);
    }

    #[test]
    fn receive_asked_waits_with_nothing_and_is_given_once() {
        let opened = OpenedLink::default();
        opened.asked();
        assert!(opened.waiting());
        assert_eq!(opened.take(), Some(Opened::Asked));
        assert_eq!(opened.take(), None);
    }

    #[test]
    fn a_debug_print_never_shows_the_link() {
        let link = crate::receiving::clipboard::tests::a_link();
        let printed = format!("{:?}", Opened::Link { link: link.clone() });
        assert_eq!(printed, "Link { .. }");
        assert!(!printed.contains(&link[link.len() - 10..]));
    }
}
