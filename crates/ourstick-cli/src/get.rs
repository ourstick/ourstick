//! `ourstick get <ticket> [--out <folder>] [--follow]`: download a stick,
//! resuming if a previous run was interrupted, fetching only what changed
//! since the last run. Prints whether the connection is direct or through a
//! relay, and every time that changes. With `--follow`, keeps going: every new
//! version the owner shares is downloaded as it comes.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::download::{
    download, wait_for_newer, DownloadError, DownloadEvent, DownloadReport, FollowEvent,
    RelayPolicy, FOLLOW_EVERY,
};
use ourstick_core::places::check_new_stick_folder;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::{describe_network, relay_reasons, Fetcher, PathKind};
use ourstick_core::Store;
use ourstick_core::Ticket;

use crate::display::untrusted;
use crate::progress::{clock, Progress};
use crate::room::Room;
use crate::setup::{self, hex};
use ourstick_core::device::random_key;

/// How long data may arrive through the relay before `get` explains why.
const EXPLAIN_AFTER: Duration = Duration::from_secs(6);

/// This device's network and the likely reasons the data uses the relay.
fn relay_explanation(fetcher: &IrohFetcher, ticket: &Ticket, elapsed: Duration) -> Vec<String> {
    let report = fetcher.network_report();
    let mut lines = vec![format!(
        "[{}] Using the relay. Likely reasons:",
        clock(elapsed)
    )];
    for reason in relay_reasons(report.as_ref(), &ticket.address_hints) {
        lines.push(format!("        - {reason}"));
    }
    if let Some(report) = report {
        lines.push(format!(
            "        This network: {}.",
            describe_network(&report)
        ));
    }
    lines
}

pub async fn run(ticket_text: &str, output: Option<PathBuf>, follow: bool) -> Result<()> {
    let ticket: Ticket = ticket_text.parse()?;
    let stick_hex = hex(&ticket.stick_id.to_bytes());
    let output = match output {
        Some(output) => output,
        None => std::env::current_dir()?.join(format!("ourstick-{}", &stick_hex[..8])),
    };

    // A reader needs no keys of its own (a fresh node key per run, below) and
    // does not keep the ticket, so no secret store is opened: `get` runs on
    // every platform. Running `get` again with the same link resumes.
    let setup = tokio::task::spawn_blocking(setup::open_for_reading).await??;
    // Not inside an app stick's folder (removing the stick would delete the
    // downloads) nor OurStick's data folder; checked before it is made.
    let output = std::path::absolute(&output)
        .with_context(|| format!("cannot resolve {}", output.display()))?;
    check_new_stick_folder(setup.store.as_ref(), &setup.data_dir, &output).context(
        "a download may not go into, or into a folder holding, a stick of this device or \
         OurStick's data (what a sender names could land there); choose another folder with --out",
    )?;
    std::fs::create_dir_all(&output)
        .with_context(|| format!("cannot create {}", output.display()))?;
    let output = std::fs::canonicalize(&output)?;
    // One `get` of a stick at a time; it also keeps this download's room in
    // the space ledger alive (ADR 0008).
    let _download_lock = ourstick_core::locks::download_lock(&setup.data_dir, &ticket.stick_id)?;
    let room = Room::new(
        setup.store.clone(),
        setup.data_dir.clone(),
        ticket.stick_id,
        output.clone(),
    );

    let store_dir = setup.data_dir.join("joined").join(&stick_hex);
    // A reader needs no lasting identity, and must not reuse the node key: a
    // `share` running on this device at the same time uses it.
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = IrohFetcher::start(&config, &store_dir).await?;
    println!("Connecting to the owner…");

    let getter = Getter {
        fetcher: &fetcher,
        store: setup.store.clone(),
        ticket: &ticket,
        output: &output,
        started: Instant::now(),
        explained: std::sync::atomic::AtomicBool::new(false),
        room: &room,
    };
    // Ctrl-C stops here instead of killing the process, so the blob store is
    // closed cleanly and keeps everything received so far.
    let outcome = tokio::select! {
        result = getter.run(follow) => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };
    let started = getter.started;
    room.release();
    if let (Some(Err(_)), Some(refusal)) = (&outcome, room.refusal()) {
        fetcher.shutdown(true).await?;
        anyhow::bail!("not downloading: {refusal}");
    }

    match outcome {
        Some(Ok(())) => {
            // Everything is in the output folder; the blob store is no longer needed.
            fetcher.shutdown(false).await?;
            Ok(())
        }
        Some(Err(error)) => {
            // Keep partial blobs so the next run resumes.
            fetcher.shutdown(true).await?;
            println!("[{}] Stopped.", clock(started.elapsed()));
            Err(error).context("download stopped; run the same command again to resume")
        }
        None => {
            println!("[{}] Stopping…", clock(started.elapsed()));
            // A second Ctrl-C quits at once, should closing hang.
            tokio::spawn(async {
                if tokio::signal::ctrl_c().await.is_ok() {
                    std::process::exit(130);
                }
            });
            fetcher.shutdown(true).await?;
            println!(
                "[{}] Stopped. Run the same command again to resume.",
                clock(started.elapsed())
            );
            std::process::exit(130);
        }
    }
}

/// Everything one run of `get` downloads with.
struct Getter<'a> {
    fetcher: &'a IrohFetcher,
    store: std::sync::Arc<dyn Store>,
    ticket: &'a Ticket,
    output: &'a std::path::Path,
    started: Instant,
    /// Set once the reasons for the relay have been printed.
    explained: std::sync::atomic::AtomicBool,
    room: &'a Room,
}

impl Getter<'_> {
    /// One download, or with `follow` one download and then every new version.
    async fn run(&self, follow: bool) -> Result<(), DownloadError> {
        let report = self.download_once().await?;
        if !follow {
            return Ok(());
        }
        println!(
            "[{}] Following: checking for a new version every {} s. Press Ctrl-C to stop.",
            clock(self.started.elapsed()),
            FOLLOW_EVERY.as_secs()
        );
        let mut seq = report.seq;
        loop {
            let mut on_event = |event: FollowEvent<'_>| {
                for line in follow_lines(&event, self.started.elapsed()) {
                    println!("{line}");
                }
            };
            let newer =
                wait_for_newer(self.fetcher, self.ticket, seq, FOLLOW_EVERY, &mut on_event).await?;
            println!(
                "[{}] The owner shared a new version ({newer}).",
                clock(self.started.elapsed())
            );
            match self.download_once().await {
                Ok(report) => seq = report.seq,
                Err(error) => {
                    println!(
                        "[{}] Download stopped: {}. Trying again at the next check.",
                        clock(self.started.elapsed()),
                        untrusted(&error.to_string())
                    );
                    tokio::time::sleep(FOLLOW_EVERY).await;
                }
            }
        }
    }

    /// One download with progress and summary.
    async fn download_once(&self) -> Result<DownloadReport, DownloadError> {
        let mut progress = Progress::default();
        let mut on_event = |event: DownloadEvent<'_>| {
            match &event {
                DownloadEvent::Planned { bytes } => {
                    if self.room.planned(*bytes).is_break() {
                        return ControlFlow::Break(());
                    }
                }
                DownloadEvent::FileReceived { size, .. } => self.room.landed(*size),
                _ => {}
            }
            let elapsed = self.started.elapsed();
            for line in progress.on_event(event, elapsed) {
                println!("{line}");
            }
            // Still on the relay after a few seconds: say why, once.
            if !self.explained.load(std::sync::atomic::Ordering::SeqCst)
                && progress.path() == Some(PathKind::Relay)
                && elapsed >= EXPLAIN_AFTER
            {
                self.explained
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                for line in relay_explanation(self.fetcher, self.ticket, elapsed) {
                    println!("{line}");
                }
            }
            ControlFlow::Continue(())
        };
        let begun = self.started.elapsed();
        let report = download(
            self.fetcher,
            self.store.clone(),
            &SystemClock,
            self.ticket,
            self.output,
            &RelayPolicy::default(),
            &mut on_event,
        )
        .await?;
        let output = self.output.display().to_string();
        let took = self.started.elapsed().saturating_sub(begun);
        for line in Progress::summary(&report, &output, self.started.elapsed(), took) {
            println!("{line}");
        }
        Ok(report)
    }
}

/// Lines for what happens while following.
fn follow_lines(event: &FollowEvent<'_>, elapsed: Duration) -> Vec<String> {
    let at = clock(elapsed);
    match event {
        FollowEvent::Unreachable { reason } => vec![format!(
            "[{at}] The owner cannot be reached ({}); still trying.",
            untrusted(&reason.to_string())
        )],
        FollowEvent::Reachable => vec![format!("[{at}] The owner can be reached again.")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::transport::TransportError;

    #[test]
    fn follow_lines_say_what_happens() {
        let at = Duration::from_secs(61);
        assert_eq!(
            follow_lines(
                &FollowEvent::Unreachable {
                    reason: &TransportError::NoRecord
                },
                at
            ),
            vec![
                "[01:01] The owner cannot be reached (the owner has no manifest to offer \
                 for this stick yet); still trying."
            ]
        );
        assert_eq!(
            follow_lines(&FollowEvent::Reachable, at),
            vec!["[01:01] The owner can be reached again."]
        );
    }
}
