//! A receive's events turned into what the window shows.

use std::time::{Duration, Instant};

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::download::{DownloadEvent, ReceiveEvent};
use ourstick_core::StickId;
use tauri::{AppHandle, Manager};

use super::{changed, waiting_words, Receivers, Shown};

/// Turns a receive's events into what the window shows, telling it at most
/// four times a second while files land.
pub(super) struct Told {
    app: AppHandle,
    stick_id: StickId,
    downloading: tokio::sync::watch::Sender<bool>,
    last: Option<Instant>,
}

impl Told {
    pub(super) fn new(
        app: AppHandle,
        stick_id: StickId,
        downloading: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            app,
            stick_id,
            downloading,
            last: None,
        }
    }

    pub(super) fn on(&mut self, event: ReceiveEvent<'_>) {
        let receivers = self.app.state::<Receivers>();
        let now_ms = SystemClock.now_ms();
        let mut progress_only = false;
        receivers.show(self.stick_id, |shown| match event {
            ReceiveEvent::Download(DownloadEvent::Planned { bytes }) => {
                *shown = Shown {
                    state: "downloading",
                    total: bytes,
                    ..Shown::default()
                }
            }
            ReceiveEvent::Download(
                DownloadEvent::FileReceived { size, .. }
                | DownloadEvent::FileCopiedHere { size, .. },
            ) => {
                shown.done = shown.done.saturating_add(size).min(shown.total);
                progress_only = true;
            }
            ReceiveEvent::Download(DownloadEvent::WaitingForMore { .. }) => {
                shown.state = "getting_more";
            }
            ReceiveEvent::Download(_) => progress_only = true,
            ReceiveEvent::UpToDate { .. } => {
                *shown = Shown {
                    state: "up_to_date",
                    ..Shown::default()
                }
            }
            ReceiveEvent::Waiting { why, .. } => {
                let since_ms = if shown.state == "waiting" {
                    shown.since_ms
                } else {
                    Some(now_ms)
                };
                *shown = Shown {
                    state: "waiting",
                    message: Some(waiting_words(&why)),
                    since_ms,
                    ..Shown::default()
                }
            }
        });
        let state = receivers.shown(&self.stick_id).state;
        self.downloading
            .send_replace(matches!(state, "downloading"));
        let due = self
            .last
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(250));
        if !progress_only || due {
            self.last = Some(Instant::now());
            changed(&self.app);
        }
    }
}
