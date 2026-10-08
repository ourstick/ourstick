//! The first half of downloading one file: receiving it. Its path is
//! checked, its blob fetched (watching which path the data arrives on, and
//! waiting while the owner prepares it), and taken out of the store into
//! `partial`. A big file is fetched this way, alone; small ones share the
//! steps before and after the fetch with the fetch queue (ADR 0016).

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::fetch_decision::LimitSampler;
use super::files::{partial_there, target_path};
use super::not_ready::Waiting;
use super::path::ReceivingPath;
use super::relay::RelayLimiter;
use super::{cancelled, io_error, DownloadError, DownloadEvent};
use crate::blocking::run_blocking;
use crate::manifest::Entry;
use crate::transport::{Fetcher, PathKind, PeerConnection, TransportError, GO};

/// A file whose blob is here, out of the store, not placed yet.
#[derive(Debug)]
pub(super) struct Received {
    pub(super) ciphertext: PathBuf,
    pub(super) plaintext: PathBuf,
    pub(super) already_local: u64,
    pub(super) received: u64,
}

/// Fetches the blob of `entry` and takes it out of the store into `partial`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn receive_file(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    entry: &Entry,
    output_dir: &Path,
    partial: &Path,
    receiving: &mut ReceivingPath<'_>,
    limiter: &mut Option<RelayLimiter>,
    received_total: &mut u64,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Received, DownloadError> {
    let path = entry.path.as_str();
    check_place(output_dir, path).await?;
    let already_local = fetcher.local_bytes(&entry.hash).await?;
    if on_event(DownloadEvent::FileStarted {
        path,
        size: entry.size,
        already_local,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    let mut waiting: Option<Waiting> = None;
    let stats = loop {
        let mut previous = 0;
        let sampler = LimitSampler::default();
        let mut look_again = false;
        let mut on_progress = |received: u64| {
            let new_bytes = received.saturating_sub(previous);
            previous = received;
            *received_total += new_bytes;
            if let Some(check) = receiving.on_received(new_bytes, || connection.usage()) {
                if check.look_again {
                    look_again = true;
                    on_event(DownloadEvent::LookingForDirect)?;
                    return ControlFlow::Break(());
                }
                if check.changed {
                    on_event(DownloadEvent::PathChanged { path: check.kind })?;
                }
                if let Some(declined) = receiving.declined(connection.local_network_declined()) {
                    on_event(DownloadEvent::LocalNetworkDeclined { declined })?;
                }
                if let Some(limiter) = limiter.as_mut() {
                    if limiter.announce(check.kind == PathKind::Relay) {
                        on_event(DownloadEvent::RelayLimited {
                            bits_per_second: limiter.bits_per_second(),
                        })?;
                    }
                }
            }
            on_event(DownloadEvent::Progress { path, received })?;
            // ADR 0006: data through a relay is slowed to the limit, never
            // stopped. Sampled now and then: `usage` is not free.
            match limiter.as_mut() {
                Some(limiter) if sampler.due(new_bytes) => {
                    ControlFlow::Continue(limiter.wait(connection.usage().relay, Instant::now()))
                }
                _ => GO,
            }
        };
        let fetched = connection.fetch_blob(&entry.hash, &mut on_progress).await;
        match fetched {
            Ok(stats) => break stats,
            // What arrived stays; the next connection resumes it.
            Err(TransportError::Cancelled) if look_again => {
                receiving.looked();
                return Err(DownloadError::LookAgain);
            }
            Err(TransportError::NotReady) => {
                let now = Instant::now();
                if waiting.is_none() && on_event(DownloadEvent::SenderPreparing { path }).is_break()
                {
                    return Err(DownloadError::Cancelled);
                }
                let pause = waiting
                    .get_or_insert_with(|| Waiting::new(now))
                    .next_pause(now)
                    .ok_or(DownloadError::Transport(TransportError::NotReady))?;
                tokio::time::sleep(pause).await;
            }
            Err(error) => return Err(cancelled(error)),
        }
    };

    let (ciphertext, plaintext) = take_out(fetcher, entry, partial).await?;
    if on_event(DownloadEvent::FileReceived {
        path,
        size: entry.size,
        already_local: stats.already_local,
        received: stats.received,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    Ok(Received {
        ciphertext,
        plaintext,
        already_local: stats.already_local,
        received: stats.received,
    })
}

/// Early check of a file's place, so a bad path fails before the transfer;
/// checked again right before the file is moved into place.
pub(super) async fn check_place(output_dir: &Path, path: &str) -> Result<(), DownloadError> {
    let output_dir = output_dir.to_path_buf();
    let path = path.to_owned();
    run_blocking(move || target_path(&output_dir, &path)).await?;
    Ok(())
}

/// Takes the blob of `entry`, complete in the store, out into `partial`:
/// the ciphertext's file, and where its plaintext will be written.
pub(super) async fn take_out(
    fetcher: &impl Fetcher,
    entry: &Entry,
    partial: &Path,
) -> Result<(PathBuf, PathBuf), DownloadError> {
    let name = data_encoding::HEXLOWER.encode(&entry.hash);
    let ciphertext = partial.join(format!("{name}.blob"));
    let plaintext = partial.join(format!("{name}.plain"));
    for leftover in [&ciphertext, &plaintext] {
        if let Err(error) = tokio::fs::remove_file(leftover).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(io_error(leftover)(error));
            }
        }
    }
    // Not into a partial folder that is not this download's any more (an
    // unplugged disk): iroh would make it again on another disk.
    {
        let partial = partial.to_path_buf();
        run_blocking(move || partial_there(&partial)).await?;
    }
    fetcher.export_blob(&entry.hash, &ciphertext).await?;
    Ok((ciphertext, plaintext))
}
