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
    landed: Landed,
}

/// The bytes of a round that are here: whole files, and the part of the
/// file coming now (with what an earlier, stopped run kept of it), so a
/// big file moves the bar while it comes.
#[derive(Debug, Default)]
pub(super) struct Landed {
    finished: u64,
    size: u64,
    already_local: u64,
    received: u64,
}

impl Landed {
    /// Takes `event`; whether it changed what is here.
    pub(super) fn on(&mut self, event: &DownloadEvent<'_>) -> bool {
        match *event {
            DownloadEvent::Planned { .. } => *self = Self::default(),
            DownloadEvent::FileStarted {
                size,
                already_local,
                ..
            } => {
                self.size = size;
                self.already_local = already_local;
                self.received = 0;
            }
            DownloadEvent::Progress { received, .. } => self.received = received,
            // Planned, and here now: received, copied from another file, or
            // left as the folder has it. (A file already here was never
            // planned.)
            DownloadEvent::FileReceived { size, .. } => {
                self.finished = self.finished.saturating_add(size);
                self.size = 0;
                self.already_local = 0;
                self.received = 0;
            }
            DownloadEvent::FileCopiedHere { size, .. }
            | DownloadEvent::FileChangedHere { size, .. } => {
                self.finished = self.finished.saturating_add(size);
            }
            _ => return false,
        }
        true
    }

    /// The bytes here, at most `total`.
    pub(super) fn bytes(&self, total: u64) -> u64 {
        let coming = self
            .already_local
            .saturating_add(self.received)
            .min(self.size);
        self.finished.saturating_add(coming).min(total)
    }
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
            landed: Landed::default(),
        }
    }

    pub(super) fn on(&mut self, event: ReceiveEvent<'_>) {
        let receivers = self.app.state::<Receivers>();
        let now_ms = SystemClock.now_ms();
        let mut progress_only = false;
        if let ReceiveEvent::Download(download) = &event {
            self.landed.on(download);
        }
        let landed = &self.landed;
        receivers.show(self.stick_id, |shown| match event {
            ReceiveEvent::Download(DownloadEvent::Planned { bytes }) => {
                *shown = Shown {
                    state: "downloading",
                    total: bytes,
                    ..Shown::default()
                }
            }
            ReceiveEvent::Download(
                DownloadEvent::FileStarted { .. }
                | DownloadEvent::Progress { .. }
                | DownloadEvent::FileReceived { .. }
                | DownloadEvent::FileCopiedHere { .. }
                | DownloadEvent::FileChangedHere { .. },
            ) => {
                shown.done = landed.bytes(shown.total);
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
            .send_replace(matches!(state, "downloading" | "getting_more"));
        let due = self
            .last
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(250));
        if !progress_only || due {
            self.last = Some(Instant::now());
            changed(&self.app);
        }
    }
}
