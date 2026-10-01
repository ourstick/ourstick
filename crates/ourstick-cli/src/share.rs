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
    keep_suitcases, plan_share, run_pass, serve_latest, Budget, PassEvent, PassOptions,
    Publication, PARTIAL_TIMING,
};
use ourstick_core::space::SystemDisks;
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::Publisher;
use ourstick_core::watcher::{
    follow_folder, FolderEvents, NotifyEvents, Owner, PeriodicEvents, WatchEvent, WatchTiming,
};
use ourstick_core::{cipher_for, ManifestHint, Ticket};
use ourstick_core::stick_names::name_from_folder;
use ourstick_core::{locks, StickId, StickKind};

use crate::display::bytes;
use crate::qr::qr_lines;
use crate::secrets::SecretsChoice;
use crate::setup::{self, hex};

mod lines;

use lines::{catalogue_line, network_line, readers_line, relay_only_line, watch_lines};

/// How long `share` waits, once online, for the network measurement.
const NETWORK_REPORT_WAIT: Duration = Duration::from_secs(10);

/// The stick already made for `folder`, or a new one with new keys (its own
/// node key among them). Under the setup lock, so two `share` runs of one
/// folder never make two sticks.
fn stick_for(setup: &setup::Setup, folder: &Path) -> Result<StickId> {
    let _setup = locks::setup_lock(&setup.data_dir)?;
    let found = setup
        .store
        .owned_sticks()?
        .into_iter()
        .find(|stick| stick.root_path == folder);
    if let Some(stick) = found {
        if stick.kind != StickKind::Cli {
            anyhow::bail!(
                "{} is the app's stick; share it from the app",
                folder.display()
            );
        }
        return Ok(stick.stick_id);
    }
    // Never refused: made fit (ADR 0009 §3), named after its folder.
    let name = name_from_folder(
        &folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let made = device::create_stick(
        setup.store.as_ref(),
        setup.secrets.as_ref(),
        folder,
        &name,
        StickKind::Cli,
    )?;
    Ok(made.stick.stick_id)
}

pub async fn run(folder: &Path, secrets: SecretsChoice, rescan: bool) -> Result<()> {
    let started = Instant::now();
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("cannot open {}", folder.display()))?;
    let (setup, sharing_locks, keys) = tokio::task::spawn_blocking({
        let folder = folder.clone();
        move || -> Result<_> {
            let setup = setup::open(&secrets)?;
            let stick_id = stick_for(&setup, &folder)?;
            // The stick's lock first (and the device node key's for an older
            // stick), then its row again, then its keys.
            let (sharing_locks, stick) =
                locks::lock_for_sharing(&setup.data_dir, setup.store.as_ref(), &stick_id)?;
            let keys = device::load_stick(setup.store.as_ref(), setup.secrets.as_ref(), stick)?;
            Ok((setup, sharing_locks, keys))
        }
    })
    .await??;
    let StickKeys {
        stick,
        signing_key,
        stick_key,
        node_key,
    } = keys;

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

    let config = IrohConfig::new(node_key, Network::Public);
    let publisher = IrohPublisher::start(&config, stick.stick_id, &cache_dir).await?;
    // ADR 0007: files are encrypted into suitcases as readers ask for them,
    // within a budget, instead of a full encrypted copy; the budget is this
    // stick's part of the device's (the space ledger).
    let budget = {
        let cache_dir = cache_dir.clone();
        let store = setup.store.clone();
        let data_dir = setup.data_dir.clone();
        let stick_id = stick.stick_id;
        tokio::task::spawn_blocking(move || {
            Budget::claim(
                &SystemDisks,
                store.as_ref(),
                &data_dir,
                &stick_id,
                &cache_dir,
            )
        })
        .await??
    };
    println!(
        "Encrypted copies of what readers ask for take up to {} in {}.",
        bytes(budget.total()),
        cache_dir.display()
    );
    let keeper = tokio::spawn(keep_suitcases(
        publisher.clone(),
        setup.store.clone(),
        stick.stick_id,
        stick_key.clone(),
        budget,
    ));
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
    // Everything that borrows the publisher ends before it shuts down.
    let stopped: Result<()> = async {
        // After a restart: the last version at once, while the folder is read.
        let mut ticket_printed = match serve_latest(&owner).await? {
            Some(latest) => {
                print_ticket(&latest)?;
                println!("Serving the last version while the folder is read again.");
                Ok(true)
            }
            None => Ok(false),
        };
        let mut last_line = None;
        let options = PassOptions {
            rescan,
            partial: Some(PARTIAL_TIMING),
        };
        // Who downloads is told from now on, also while the folder is read:
        // readers start with the first version.
        let readers = report_readers(&publisher, started);
        tokio::pin!(readers);
        let mut on_pass = |event: PassEvent<'_>| match event {
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
            // `share <folder>` links nothing (linked entries are the app's).
            PassEvent::Roots { .. } => {}
        };
        let outcome = tokio::select! {
            outcome = run_pass(&owner, options, &mut on_pass) => outcome?,
            () = &mut readers => anyhow::bail!("the reader activity ended unexpectedly"),
        };
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
        match NotifyEvents::start(&folder) {
            Ok(events) => {
                println!("Watching the folder: changes are shared as they happen.");
                serve(&owner, events, current, readers, started).await
            }
            Err(error) => {
                println!(
                    "{error}. Checking the folder every {} s instead.",
                    RESCAN_EVERY.as_secs()
                );
                let events = PeriodicEvents::new(RESCAN_EVERY);
                serve(&owner, events, current, readers, started).await
            }
        }
    }
    .await;
    stopped?;
    println!("Stopping…");
    publisher.shutdown().await;
    match keeper.await {
        Ok(Err(error)) => println!("The suitcases' keeper stopped: {error}"),
        Err(error) => println!("The suitcases' keeper stopped: {error}"),
        Ok(Ok(())) => {}
    }
    // Only now may another sharer start.
    drop(sharing_locks);
    drop(setup.key_file_lock);
    Ok(())
}

/// Prints who downloads, and whether they only come through the relay, as
/// it changes. Ends only if the publisher stops reporting.
async fn report_readers(publisher: &IrohPublisher, started: Instant) {
    let mut activity = publisher.activity();
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
}

/// How often the folder is checked when the system cannot report changes.
const RESCAN_EVERY: Duration = Duration::from_secs(60);

/// Follows the folder until Ctrl-C, printing what changes.
async fn serve(
    owner: &Owner<'_, IrohPublisher>,
    mut events: impl FolderEvents,
    current: ourstick_core::Manifest,
    readers: std::pin::Pin<&mut impl std::future::Future<Output = ()>>,
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
    tokio::select! {
        () = follow_folder(owner, &mut events, timing, current, &mut on_event) => {
            anyhow::bail!("the folder watch ended unexpectedly")
        }
        () = readers => anyhow::bail!("the reader activity ended unexpectedly"),
        result = tokio::signal::ctrl_c() => Ok(result?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::secrets::MemorySecretStore;
    use ourstick_core::store::MemoryStore;
    use std::sync::Arc;

    fn setup(data_dir: &Path) -> setup::Setup {
        setup::Setup {
            key_file_lock: None,
            data_dir: data_dir.to_path_buf(),
            store: Arc::new(MemoryStore::new()),
            secrets: Box::new(MemorySecretStore::new()),
        }
    }

    #[test]
    fn a_folder_gets_one_stick_and_never_the_apps() {
        let dir = tempfile::tempdir().unwrap();
        let setup = setup(dir.path());
        let first = stick_for(&setup, Path::new("/photos")).unwrap();
        assert_eq!(stick_for(&setup, Path::new("/photos")).unwrap(), first);
        let stick = setup.store.owned_stick(&first).unwrap().unwrap();
        assert_eq!(
            (stick.name.as_str(), stick.kind),
            ("photos", StickKind::Cli)
        );

        device::create_stick(
            setup.store.as_ref(),
            setup.secrets.as_ref(),
            Path::new("/drive"),
            "OurStick",
            StickKind::App,
        )
        .unwrap();
        let refused = stick_for(&setup, Path::new("/drive")).unwrap_err();
        assert!(refused.to_string().contains("the app's stick"), "{refused}");
    }

    #[test]
    fn a_stick_is_not_found_or_made_while_another_program_sets_up() {
        let dir = tempfile::tempdir().unwrap();
        let setup = setup(dir.path());
        let _held = locks::setup_lock(dir.path()).unwrap();
        let refused = stick_for(&setup, Path::new("/photos")).unwrap_err();
        assert!(refused.downcast_ref::<locks::LockError>().is_some());
        assert!(setup.store.owned_sticks().unwrap().is_empty());
    }
}
