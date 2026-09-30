//! `ourstick share <folder>`: catalogue the folder and encrypt it into the
//! cache, print a ticket (on a first share, as soon as the first version is
//! out), and serve until Ctrl-C.

use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::device::{self, StickKeys};
use ourstick_core::share::{
    plan_share, run_pass, PassEvent, PassOptions, Publication, PARTIAL_TIMING,
};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::Publisher;
use ourstick_core::watcher::{
    follow_folder, FolderEvents, NotifyEvents, Owner, PeriodicEvents, WatchEvent, WatchTiming,
};
use ourstick_core::{cipher_for, ManifestHint, Ticket};

use crate::display::bytes;
use crate::qr::qr_lines;
use crate::secrets::SecretsChoice;
use crate::setup::{self, hex};

mod lines;

use lines::{catalogue_line, network_line, readers_line, relay_only_line, watch_lines};

/// How long `share` waits, once online, for the network measurement.
const NETWORK_REPORT_WAIT: Duration = Duration::from_secs(10);

/// Finds the stick already made for `folder`, or makes one with new keys.
fn owned_stick_for(setup: &setup::Setup, folder: &Path) -> Result<StickKeys> {
    let found = setup
        .store
        .owned_sticks()?
        .into_iter()
        .find(|stick| stick.root_path == folder);
    if let Some(stick) = found {
        return Ok(device::load_stick(setup.secrets.as_ref(), stick)?);
    }
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stick".to_owned());
    Ok(device::create_stick(
        setup.store.as_ref(),
        setup.secrets.as_ref(),
        folder,
        &name,
        None,
    )?)
}

pub async fn run(folder: &Path, secrets: SecretsChoice, rescan: bool) -> Result<()> {
    let started = Instant::now();
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("cannot open {}", folder.display()))?;
    let (setup, stick, signing_key, stick_key) = tokio::task::spawn_blocking({
        let folder = folder.clone();
        move || -> Result<_> {
            let setup = setup::open(&secrets)?;
            let StickKeys {
                stick,
                signing_key,
                stick_key,
            } = owned_stick_for(&setup, &folder)?;
            Ok((setup, stick, signing_key, stick_key))
        }
    })
    .await??;

    let cache_dir = setup
        .data_dir
        .join("cache")
        .join(hex(&stick.stick_id.to_bytes()));
    let plan = {
        let folder = folder.clone();
        let cipher = cipher_for(stick.cipher, &stick_key);
        tokio::task::spawn_blocking(move || plan_share(&folder, cipher.as_ref())).await??
    };
    println!(
        "Sharing {} ({} files, {}).",
        folder.display(),
        plan.files,
        bytes(plan.plaintext_bytes)
    );
    println!(
        "The encrypted cache will use about {} in {}",
        bytes(plan.cache_bytes),
        cache_dir.display()
    );

    let config = IrohConfig::new(setup.node_key, Network::Public);
    let publisher = IrohPublisher::start(&config, stick.stick_id, &cache_dir).await?;
    // Before the first version, so the ticket carries the relay's address.
    if !publisher.online(Duration::from_secs(30)).await {
        println!("Warning: no relay reachable; the ticket only has direct addresses.");
    }
    let report = publisher.wait_network_report(NETWORK_REPORT_WAIT).await;
    println!("{}", network_line(report.as_ref()));

    let owner = Owner {
        publisher: &publisher,
        store: setup.store.clone(),
        clock: &SystemClock,
        signing_key: &signing_key,
        stick_key: &stick_key,
    };
    println!("Reading the folder…");
    let print_ticket = |publication: &Publication| -> Result<()> {
        let ticket = Ticket {
            stick_id: stick.stick_id,
            node_id: publisher.node_id(),
            manifest_hint: ManifestHint {
                seq: publication.seq,
                hash: publication.manifest_hash,
            },
            address_hints: publisher.address_hints(),
            stick_key: stick_key.clone(),
        };
        // Printed for the user only; never logged. The QR code only on a
        // terminal, where a phone can scan it; not into files or pipes.
        let link = ticket.to_link()?;
        println!("\nTicket (anyone with it can read this stick):\n\n{link}\n");
        if std::io::stdout().is_terminal() {
            for line in qr_lines(&link)? {
                println!("{line}");
            }
            println!();
        }
        Ok(())
    };
    let mut ticket_printed = Ok(false);
    let mut last_line = None;
    let options = PassOptions {
        rescan,
        partial: Some(PARTIAL_TIMING),
    };
    let outcome = run_pass(&owner, options, &mut |event| match event {
        PassEvent::Published(publication) => {
            if matches!(ticket_printed, Ok(false)) {
                ticket_printed = print_ticket(publication).map(|()| true);
                println!("Readers can start now; the rest follows as the folder is read.");
            }
        }
        PassEvent::Progress { done, total } => {
            if let Some(line) = catalogue_line(&mut last_line, done, total, started.elapsed()) {
                println!("{line}");
            }
        }
    })
    .await?;
    let publication = outcome.publication;
    if !ticket_printed? {
        print_ticket(&publication)?;
    }
    if publication.reused > 0 {
        println!(
            "{} files were as last time and were not read again \
             (--rescan reads every file).",
            publication.reused
        );
    }
    if !publication.new_version {
        println!("No changes since the last share: readers keep the same version.");
    }
    if outcome.skipped > 0 {
        println!(
            "{} files changed while they were read; they follow shortly.",
            outcome.skipped
        );
    }
    println!("Serving. Press Ctrl-C to stop.");

    let current = publication.manifest;
    let stopped = match NotifyEvents::start(&folder) {
        Ok(events) => {
            println!("Watching the folder: changes are shared as they happen.");
            serve(&owner, &folder, events, current, started).await
        }
        Err(error) => {
            println!(
                "{error}. Checking the folder every {} s instead.",
                RESCAN_EVERY.as_secs()
            );
            let events = PeriodicEvents::new(RESCAN_EVERY);
            serve(&owner, &folder, events, current, started).await
        }
    };
    stopped?;
    println!("Stopping…");
    publisher.shutdown().await;
    // Only now may another sharer start.
    drop(setup.lock);
    Ok(())
}

/// How often the folder is checked when the system cannot report changes.
const RESCAN_EVERY: Duration = Duration::from_secs(60);

/// Follows the folder until Ctrl-C, printing what changes.
async fn serve(
    owner: &Owner<'_, IrohPublisher>,
    folder: &Path,
    mut events: impl FolderEvents,
    current: ourstick_core::Manifest,
    started: Instant,
) -> Result<()> {
    let timing = WatchTiming::default();
    // Reading the folder is told only for a pass that takes a while.
    let mut last_reading = None;
    let mut done_before = usize::MAX;
    let mut on_event = |event: WatchEvent<'_>| {
        if let WatchEvent::Reading { done, total } = event {
            if done <= done_before {
                // A new pass: its first line comes after a while, if at all.
                last_reading = Some(started.elapsed());
            }
            done_before = done;
            if done < total {
                if let Some(line) =
                    catalogue_line(&mut last_reading, done, total, started.elapsed())
                {
                    println!("{line}");
                }
            }
        }
        for line in watch_lines(&event, started.elapsed(), timing) {
            println!("{line}");
        }
    };
    let readers = async {
        let mut activity = owner.publisher.activity();
        let mut before = 0;
        let mut relay_only_said = false;
        while activity.changed().await.is_ok() {
            let now = *activity.borrow();
            if let Some(line) = readers_line(before, &now, started.elapsed()) {
                println!("{line}");
            }
            if let Some(line) = relay_only_line(relay_only_said, &now, started.elapsed()) {
                println!("{line}");
            }
            before = now.readers;
            relay_only_said |= now.only_relayed;
        }
    };
    tokio::select! {
        () = follow_folder(owner, folder, &mut events, timing, current, &mut on_event) => {
            anyhow::bail!("the folder watch ended unexpectedly")
        }
        () = readers => anyhow::bail!("the reader activity ended unexpectedly"),
        result = tokio::signal::ctrl_c() => Ok(result?),
    }
}
