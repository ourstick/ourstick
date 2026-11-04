//! A receive's events turned into what the window shows.

use std::time::{Duration, Instant};

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::download::{Cancel, DownloadEvent, ReceiveEvent};
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
    /// Tells the owner how far this is (ADR 0015), from the same bytes the
    /// bar shows.
    reporter: Option<std::sync::Arc<ourstick_core::download::Reporter>>,
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
            // Planned, and here now: received, or copied from another file.
            // A file left as the folder has it is not counted: most were
            // never planned (the user changed them there), and a bar a little
            // short before "Up to date" is better than one full too soon.
            DownloadEvent::FileReceived { size, .. } => {
                self.finished = self.finished.saturating_add(size);
                self.size = 0;
                self.already_local = 0;
                self.received = 0;
            }
            DownloadEvent::FileCopiedHere { size, .. } => {
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
        reporter: Option<std::sync::Arc<ourstick_core::download::Reporter>>,
    ) -> Self {
        Self {
            app,
            stick_id,
            downloading,
            last: None,
            landed: Landed::default(),
            reporter,
        }
    }

    /// Takes `event`; on a phone, a wait ends the receive (`cancel`).
    pub(super) fn on(&mut self, event: ReceiveEvent<'_>, cancel: &Cancel) {
        if !super::FOLLOWS && matches!(event, ReceiveEvent::Waiting { .. }) {
            cancel.cancel();
        }
        let receivers = self.app.state::<Receivers>();
        let now_ms = SystemClock.now_ms();
        let mut progress_only = false;
        if let ReceiveEvent::Download(download) = &event {
            self.landed.on(download);
        }
        if let Some(reporter) = &self.reporter {
            let total = match &event {
                ReceiveEvent::Download(DownloadEvent::Planned { bytes, .. }) => *bytes,
                _ => receivers.shown(&self.stick_id).total,
            };
            reporter.on_event(&event, self.landed.bytes(total), total);
        }
        match &event {
            ReceiveEvent::Download(DownloadEvent::Planned { .. }) => {
                receivers.round_planned(self.stick_id)
            }
            ReceiveEvent::Download(DownloadEvent::Contents { manifest, .. }) => {
                receivers.contents_now(self.stick_id, manifest)
            }
            ReceiveEvent::Download(DownloadEvent::SavedAs {
                path,
                saved_as,
                files,
            }) => receivers.saved_as_too(
                self.stick_id,
                ourstick_core::download::SavedAs {
                    path: (*path).to_owned(),
                    saved_as: (*saved_as).to_owned(),
                    files: *files,
                },
            ),
            _ => {}
        }
        let landed = &self.landed;
        receivers.show(self.stick_id, |shown| match event {
            ReceiveEvent::Download(DownloadEvent::Planned { bytes, .. }) => {
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
                | DownloadEvent::FileCopiedHere { .. },
            ) => {
                shown.done = landed.bytes(shown.total);
                progress_only = true;
            }
            ReceiveEvent::Download(DownloadEvent::LocalNetworkDeclined { declined }) => {
                shown.local_network_declined = declined;
            }
            ReceiveEvent::Download(DownloadEvent::WaitingForMore { .. }) => {
                shown.state = "getting_more";
            }
            ReceiveEvent::Download(_) => progress_only = true,
            ReceiveEvent::UpToDate { .. } if super::FOLLOWS => {
                *shown = Shown {
                    state: "up_to_date",
                    ..Shown::default()
                }
            }
            // Not followed: all here when it was checked.
            ReceiveEvent::UpToDate { .. } => {
                *shown = Shown {
                    state: "checked",
                    since_ms: Some(now_ms),
                    ..Shown::default()
                }
            }
            // A phone does not wait (its receive ends): tried again when
            // the user asks.
            ReceiveEvent::Waiting { why, .. } if !super::FOLLOWS => {
                *shown = Shown {
                    state: "stopped",
                    message: Some(waiting_words(&why)),
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
