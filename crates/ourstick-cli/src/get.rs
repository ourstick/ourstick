//! `ourstick get <ticket> [--out <folder>]`: download a stick, resuming if a
//! previous run was interrupted. Prints whether the connection is direct or
//! through a relay, and every time that changes.

use std::ops::ControlFlow;
use std::path::PathBuf;

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::download::{download, DownloadEvent};
use ourstick_core::secrets::SecretName;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::{Fetcher, PathKind};
use ourstick_core::Ticket;

use crate::display::{bytes, untrusted};
use crate::setup::{self, hex, random_key};

fn describe(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "direct connection",
        PathKind::Relay => "through a relay (slower; a direct path may still be found)",
        PathKind::Unknown => "connection type not known yet",
    }
}

pub async fn run(ticket_text: &str, output: Option<PathBuf>) -> Result<()> {
    let ticket: Ticket = ticket_text.parse()?;
    let stick_hex = hex(&ticket.stick_id.to_bytes());
    let output = match output {
        Some(output) => output,
        None => std::env::current_dir()?.join(format!("ourstick-{}", &stick_hex[..8])),
    };
    std::fs::create_dir_all(&output)
        .with_context(|| format!("cannot create {}", output.display()))?;
    let output = std::fs::canonicalize(&output)?;

    let setup = {
        let ticket_text = ticket_text.trim().to_owned();
        let stick_id = ticket.stick_id;
        tokio::task::spawn_blocking(move || -> Result<_> {
            let setup = setup::open()?;
            // A reader keeps its ticket as a secret: it holds the stick key.
            setup
                .secrets
                .put(&SecretName::Ticket(stick_id), ticket_text.as_bytes())?;
            Ok(setup)
        })
        .await??
    };

    let store_dir = setup.data_dir.join("joined").join(&stick_hex);
    // A reader needs no lasting identity, and must not reuse the node key: a
    // `share` running on this device at the same time uses it.
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = IrohFetcher::start(&config, &store_dir).await?;
    println!("Connecting to the owner…");

    let mut on_event = |event: DownloadEvent<'_>| {
        match event {
            DownloadEvent::Connected { path } => println!("Connected: {}.", describe(path)),
            DownloadEvent::PathChanged { path } => println!("Now: {}.", describe(path)),
            DownloadEvent::Manifest {
                files,
                bytes: total,
                unchanged,
                ..
            } => {
                let note = if unchanged {
                    " (no changes since last time)"
                } else {
                    ""
                };
                println!("{files} files, {}{note}.", bytes(total));
            }
            DownloadEvent::FileDone {
                path,
                already_local,
                received,
            } => {
                let resumed = if already_local > 0 {
                    format!(", resumed after {}", bytes(already_local))
                } else {
                    String::new()
                };
                println!(
                    "  {} ({} received{resumed})",
                    untrusted(path),
                    bytes(received)
                );
            }
            DownloadEvent::FileStarted { .. } | DownloadEvent::Progress { .. } => {}
        }
        ControlFlow::Continue(())
    };
    let result = download(
        &fetcher,
        setup.store.clone(),
        &SystemClock,
        &ticket,
        &output,
        &mut on_event,
    )
    .await;

    match result {
        Ok(report) => {
            // Everything is in the output folder; the blob store is no longer needed.
            fetcher.shutdown(false).await?;
            println!(
                "Done: {} files in {} ({} received over the network).",
                report.files,
                output.display(),
                bytes(report.received)
            );
            Ok(())
        }
        Err(error) => {
            // Keep partial blobs so the next run resumes.
            fetcher.shutdown(true).await?;
            Err(error).context("download stopped; run the same command again to resume")
        }
    }
}
