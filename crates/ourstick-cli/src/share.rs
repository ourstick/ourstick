//! `ourstick share <folder>`: encrypt the folder into the cache, print a
//! ticket, and serve until Ctrl-C.

use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::secrets::SecretName;
use ourstick_core::share::{plan_share, publish_folder};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::{describe_network, NetworkReport, Publisher};
use ourstick_core::watcher::{
    follow_folder, FolderEvents, NotifyEvents, Owner, PeriodicEvents, WatchEvent, WatchTiming,
};
use ourstick_core::{
    cipher_for, CipherId, ManifestHint, OwnedStick, StickKey, StickSigningKey, Ticket,
};

use crate::display::{bytes, untrusted};
use crate::progress::clock;
use crate::qr::qr_lines;
use crate::secrets::SecretsChoice;
use crate::setup::{self, hex, load_key, now_ms};

const CIPHER: CipherId = CipherId::XChaCha20Poly1305;
/// How long `share` waits, once online, for the network measurement.
const NETWORK_REPORT_WAIT: Duration = Duration::from_secs(10);

/// The "This network" line: what was measured, or that it is not known yet;
/// never nothing.
fn network_line(report: Option<&NetworkReport>) -> String {
    match report {
        Some(report) => format!("This network: {}.", describe_network(report)),
        None => "This network: not measured yet (readers still connect; \
                 `get` explains the path it uses)."
            .to_owned(),
    }
}

/// Finds the stick already made for `folder`, or makes one with new keys.
fn owned_stick_for(
    setup: &setup::Setup,
    folder: &Path,
) -> Result<(OwnedStick, StickSigningKey, StickKey)> {
    if let Some(stick) = setup
        .store
        .owned_sticks()?
        .into_iter()
        .find(|stick| stick.root_path == folder)
    {
        let signing = load_key(
            setup.secrets.as_ref(),
            &SecretName::StickSigningKey(stick.stick_id),
        )?;
        let stick_key = load_key(
            setup.secrets.as_ref(),
            &SecretName::StickKey(stick.stick_id),
        )?;
        return Ok((
            stick,
            StickSigningKey::from_bytes(&signing),
            StickKey::from_bytes(stick_key),
        ));
    }
    let signing_key = StickSigningKey::generate();
    let stick_key = StickKey::generate();
    let stick_id = signing_key.stick_id();
    // Keys first: a stick in the database must always have its keys.
    setup.secrets.put(
        &SecretName::StickSigningKey(stick_id),
        &signing_key.to_bytes(),
    )?;
    setup
        .secrets
        .put(&SecretName::StickKey(stick_id), stick_key.as_bytes())?;
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stick".to_owned());
    let stick = OwnedStick::new(stick_id, name, folder, CIPHER, now_ms());
    setup.store.add_owned_stick(&stick)?;
    Ok((stick, signing_key, stick_key))
}

pub async fn run(folder: &Path, secrets: SecretsChoice, rescan: bool) -> Result<()> {
    let started = Instant::now();
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("cannot open {}", folder.display()))?;
    let (setup, stick, signing_key, stick_key) = tokio::task::spawn_blocking({
        let folder = folder.clone();
        move || -> Result<_> {
            let setup = setup::open(&secrets)?;
            let (stick, signing_key, stick_key) = owned_stick_for(&setup, &folder)?;
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
    println!("Encrypting…");
    let publication = publish_folder(
        &publisher,
        setup.store.clone(),
        &SystemClock,
        &signing_key,
        &stick_key,
        rescan,
    )
    .await?;
    if publication.reused > 0 {
        println!(
            "{} files were as last time and were not encrypted again \
             (--rescan reads every file).",
            publication.reused
        );
    }
    if !publication.new_version {
        println!("No changes since the last share: readers keep the same version.");
    }
    if !publisher.online(Duration::from_secs(30)).await {
        println!("Warning: no relay reachable; the ticket only has direct addresses.");
    }
    let report = publisher.wait_network_report(NETWORK_REPORT_WAIT).await;
    println!("{}", network_line(report.as_ref()));

    let ticket = Ticket {
        stick_id: stick.stick_id,
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: publication.seq,
            hash: publication.manifest_hash,
        },
        address_hints: publisher.address_hints(),
        stick_key,
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
    println!("Serving. Press Ctrl-C to stop.");

    let owner = Owner {
        publisher: &publisher,
        store: setup.store.clone(),
        clock: &SystemClock,
        signing_key: &signing_key,
        // The ticket holds the stick key now.
        stick_key: &ticket.stick_key,
    };
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
    let mut on_event = |event: WatchEvent<'_>| {
        for line in watch_lines(&event, started.elapsed(), timing) {
            println!("{line}");
        }
    };
    tokio::select! {
        () = follow_folder(owner, folder, &mut events, timing, current, &mut on_event) => {
            anyhow::bail!("the folder watch ended unexpectedly")
        }
        result = tokio::signal::ctrl_c() => Ok(result?),
    }
}

/// Lines for what the watcher did. Nothing when signs of change changed
/// nothing (e.g. a file saved with the same content).
fn watch_lines(event: &WatchEvent<'_>, elapsed: Duration, timing: WatchTiming) -> Vec<String> {
    let at = clock(elapsed);
    match event {
        WatchEvent::Unchanged => Vec::new(),
        WatchEvent::Ejected => vec![format!(
            "[{at}] The stick's drive is not there (ejected?); readers keep getting \
             the last version until it is back."
        )],
        WatchEvent::StillChanging => {
            vec![format!("[{at}] A file is still being written; waiting…")]
        }
        WatchEvent::Failed { error } => vec![format!(
            "[{at}] Could not share the change: {}. Trying again at the next change, \
             or in {} s.",
            untrusted(&error.to_string()),
            timing.retry.as_secs()
        )],
        WatchEvent::Published {
            changes,
            publication,
        } => {
            let mut parts = Vec::new();
            for (count, what) in [
                (changes.added.len(), "added"),
                (changes.changed.len(), "changed"),
                (changes.removed.len(), "removed"),
            ] {
                if count > 0 {
                    parts.push(format!("{count} {what}"));
                }
            }
            let mut lines = vec![format!(
                "[{at}] Folder changed: {}. Readers now get version {}.",
                parts.join(", "),
                publication.seq
            )];
            let named = changes
                .added
                .iter()
                .map(|path| ("+", path))
                .chain(changes.changed.iter().map(|path| ("~", path)))
                .chain(changes.removed.iter().map(|path| ("-", path)));
            let total = changes.added.len() + changes.changed.len() + changes.removed.len();
            for (mark, path) in named.take(MAX_NAMED) {
                lines.push(format!("        {mark} {}", untrusted(path)));
            }
            if total > MAX_NAMED {
                lines.push(format!("        … and {} more", total - MAX_NAMED));
            }
            lines
        }
    }
}

/// At most this many changed paths are listed after a change.
const MAX_NAMED: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    fn publication(seq: u64) -> ourstick_core::share::Publication {
        let key = ourstick_core::StickSigningKey::from_bytes(&[1; 32]);
        ourstick_core::share::Publication {
            manifest: ourstick_core::Manifest::empty(),
            manifest_hash: [0; 32],
            seq,
            record: key.sign_record(seq, [0; 32]),
            new_version: true,
            reused: 0,
        }
    }

    #[test]
    fn watch_lines_say_what_changed_briefly_and_safely() {
        let timing = WatchTiming::default();
        let at = Duration::from_secs(75);
        let changes = ourstick_core::watcher::ChangeSummary {
            added: (1..=6).map(|index| format!("new{index}.txt")).collect(),
            changed: vec!["\u{1b}[31mred.txt".to_owned()],
            removed: vec![],
        };
        let publication = publication(42);
        let lines = watch_lines(
            &WatchEvent::Published {
                changes: &changes,
                publication: &publication,
            },
            at,
            timing,
        );
        assert_eq!(
            lines,
            vec![
                "[01:15] Folder changed: 6 added, 1 changed. Readers now get version 42.",
                "        + new1.txt",
                "        + new2.txt",
                "        + new3.txt",
                "        + new4.txt",
                "        + new5.txt",
                "        … and 2 more",
            ]
        );
        assert!(watch_lines(&WatchEvent::Unchanged, at, timing).is_empty());
        let failed = watch_lines(
            &WatchEvent::Failed {
                error: &ourstick_core::share::ShareError::NotOwned,
            },
            at,
            timing,
        );
        assert_eq!(
            failed,
            vec![
                "[01:15] Could not share the change: this device does not own that stick. \
                 Trying again at the next change, or in 30 s."
            ]
        );
    }

    #[test]
    fn changed_names_are_escaped() {
        let changes = ourstick_core::watcher::ChangeSummary {
            added: vec![],
            changed: vec!["\u{1b}[31mred.txt".to_owned()],
            removed: vec![],
        };
        let publication = publication(1);
        let lines = watch_lines(
            &WatchEvent::Published {
                changes: &changes,
                publication: &publication,
            },
            Duration::ZERO,
            WatchTiming::default(),
        );
        assert_eq!(lines[1], "        ~ \\u{1b}[31mred.txt");
    }

    #[test]
    fn network_line_is_never_empty() {
        assert!(network_line(None).starts_with("This network: not measured yet"));
        let report = NetworkReport {
            udp_v4: true,
            udp_v6: false,
            ipv4_mapping_varies: Some(false),
            public_v4: Some("203.0.113.32:35611".parse().unwrap()),
            public_v6: None,
        };
        assert_eq!(
            network_line(Some(&report)),
            format!("This network: {}.", describe_network(&report))
        );
    }
}
