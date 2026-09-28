//! `ourstick get <ticket> [--out <folder>]`: download a stick, resuming if a
//! previous run was interrupted. Prints whether the connection is direct or
//! through a relay, and every time that changes.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::download::{download, DownloadEvent};
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::{describe_network, relay_reasons, Fetcher, PathKind};
use ourstick_core::Ticket;

use crate::progress::{clock, Progress};

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
use crate::setup::{self, hex, random_key};

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

    // A reader needs no keys of its own (a fresh node key per run, below) and
    // does not keep the ticket, so no secret store is opened: `get` runs on
    // every platform. Running `get` again with the same link resumes.
    let setup = tokio::task::spawn_blocking(setup::open_for_reading).await??;

    let store_dir = setup.data_dir.join("joined").join(&stick_hex);
    // A reader needs no lasting identity, and must not reuse the node key: a
    // `share` running on this device at the same time uses it.
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = IrohFetcher::start(&config, &store_dir).await?;
    println!("Connecting to the owner…");

    let started = Instant::now();
    let mut progress = Progress::default();
    let mut explained = false;
    let mut on_event = |event: DownloadEvent<'_>| {
        let elapsed = started.elapsed();
        for line in progress.on_event(event, elapsed) {
            println!("{line}");
        }
        // Still on the relay after a few seconds: say why, once.
        if !explained && progress.path() == Some(PathKind::Relay) && elapsed >= EXPLAIN_AFTER {
            explained = true;
            for line in relay_explanation(&fetcher, &ticket, elapsed) {
                println!("{line}");
            }
        }
        ControlFlow::Continue(())
    };
    // Ctrl-C stops the download here instead of killing the process, so the
    // blob store is closed cleanly and keeps everything received so far.
    let outcome = tokio::select! {
        result = download(
            &fetcher,
            setup.store.clone(),
            &SystemClock,
            &ticket,
            &output,
            &mut on_event,
        ) => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };

    match outcome {
        Some(Ok(report)) => {
            // Everything is in the output folder; the blob store is no longer needed.
            fetcher.shutdown(false).await?;
            let output = output.display().to_string();
            for line in Progress::summary(&report, &output, started.elapsed()) {
                println!("{line}");
            }
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
