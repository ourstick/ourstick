//! The reader's side: fetch and check the latest record, fetch and decrypt the
//! manifest, then fetch, decrypt and write every file, batch by batch.
//! Interrupted downloads resume, because the fetcher keeps partial blobs of
//! the batch in progress.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

mod batches;
mod entry;
mod fetch_files;
pub(crate) mod files;
mod folder;
mod follow;
mod joined;
mod left_out;
mod listing;
mod not_ready;
mod path;
mod placing_queue;
mod receive;
mod relay;
mod relay_watch;
mod reporter;
pub(crate) mod saved_lines;
pub(crate) mod saved_names;
pub(crate) mod saved_paths;
mod types;

use fetch_files::{fetch_files, FileCounts};
use joined::{record_downloaded, RECORD_GROUP};
use listing::fetch_listing;
use path::ReceivingPath;
use relay::RelayLimiter;

pub use batches::SCRATCH_BYTES;
pub use follow::{wait_for_newer, FollowEvent, FOLLOW_EVERY};
pub use left_out::{LeftOut, PathProblem, Tick};
pub use listing::{entries_in, list, preview, Listing, Preview, TopEntry};
pub use receive::*;
pub use relay::RelayPolicy;
pub use reporter::{wait_reason, NotSent, ReportTo, Reporter, REPORT_DEADLINE, REPORT_EVERY};
pub use saved_lines::saved_elsewhere;
pub use saved_paths::SavedAs;
pub use types::{DownloadError, DownloadEvent, DownloadReport};

/// How many new versions a download follows after a refusal before giving up;
/// a folder that never stops changing cannot keep it going forever. Versions
/// of a catalogue that is still growing are not counted.
const MAX_NEW_VERSIONS: u32 = 10;

/// How a connection of a download ended.
enum Over {
    Done(DownloadReport),
    /// Every file of this partial version is in place; more will come.
    Partial {
        seq: u64,
    },
}

/// What carries over from one connection of a download to the next.
#[derive(Debug, Default)]
struct Carried {
    /// Blob bytes received, over all connections (a stopped fetch counts too).
    received: u64,
    /// The version the last connection was downloading, once known.
    seq: Option<u64>,
    /// The owner refused data of this version (`None`: before it was known):
    /// the next connection continues only if the owner has a newer one.
    refused: Option<(Option<u64>, DownloadError)>,
    /// How long the download has been on the relay, over connections.
    watch: relay_watch::RelayWatch,
    /// Connections lost in a row with nothing received in between.
    lost: u32,
    /// `received` when the last connection was lost.
    received_at_loss: u64,
}

/// Connections lost in a row, with nothing received in between, before the
/// download stops: an owner that answers and then drops every connection.
const MAX_LOST_IN_A_ROW: u32 = 5;

use crate::blocking::run_blocking;
use crate::clock::Clock;
use crate::store::{JoinedStick, ManifestVersion, StickKind, Store, StoreError};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PathUsage, PeerConnection, TransportError};

/// Where partially written files live inside the output folder, so the final
/// rename stays on one file system. Removed at the end.
const PARTIAL_DIR: &str = ".ourstick-partial";

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> DownloadError + '_ {
    move |source| DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn cancelled(error: TransportError) -> DownloadError {
    match error {
        TransportError::Cancelled => DownloadError::Cancelled,
        other => DownloadError::Transport(other),
    }
}

/// Downloads the stick named in `ticket` into `output_dir`.
///
/// Records the stick as joined on first use, with `output_dir` as its folder
/// (the latest folder wins), raises its record floor (ADR 0004), and records
/// the version once all its files are in `output_dir`. On any error nothing half-written is left under a final name; run
/// again to resume. The caller shuts the fetcher down afterwards (keeping its
/// store on error, so the next run resumes).
/// Data through a relay is limited as `policy` says (ADR 0006); the download
/// never stops or asks because of the relay.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    ticket: &Ticket,
    output_dir: &Path,
    policy: &RelayPolicy,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    let kind = StickKind::Cli;
    download_as(
        kind, fetcher, store, clock, ticket, output_dir, policy, on_event,
    )
    .await
}

/// [`download`] for the program `kind`: the window (`App`) takes over a stick
/// `get` received; `get` is refused the window's ([`DownloadError::ReceivedByApp`]).
#[allow(clippy::too_many_arguments)]
pub async fn download_as(
    kind: StickKind,
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    ticket: &Ticket,
    output_dir: &Path,
    policy: &RelayPolicy,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    let stick_id = ticket.stick_id;
    joined::prepare(&store, stick_id, output_dir, kind, clock.now_ms()).await?;

    let mut total = PathUsage::default();
    let mut carried = Carried::default();
    let mut new_versions = 0;
    loop {
        // Read again on every attempt: an earlier attempt may have raised
        // the floor.
        let (joined, floor) = {
            let store = store.clone();
            run_blocking(move || {
                let joined = store
                    .joined_stick(&stick_id)?
                    .ok_or(StoreError::StickNotFound)?;
                Ok::<_, StoreError>((joined, store.record_floor(&stick_id)?))
            })
            .await?
        };
        let connection = fetcher.connect(ticket).await?;
        let result = download_over(
            fetcher,
            &connection,
            store.clone(),
            ticket,
            &joined,
            floor,
            output_dir,
            policy,
            &mut carried,
            on_event,
        )
        .await;
        let usage = connection.usage();
        total.direct += usage.direct;
        total.relay += usage.relay;
        total.direct_ipv6 += usage.direct_ipv6;
        connection.close().await;
        match result {
            Ok(Over::Done(report)) => {
                return Ok(DownloadReport {
                    received: carried.received,
                    usage: total,
                    ..report
                })
            }
            // ADR 0007: the owner is still reading its folder. Wait for its
            // next version, then go on with it.
            Ok(Over::Partial { seq }) => {
                if on_event(DownloadEvent::WaitingForMore { seq }).is_break() {
                    return Err(DownloadError::Cancelled);
                }
                let mut cancelled = false;
                wait_for_newer(fetcher, ticket, seq, FOLLOW_EVERY, &mut |event| {
                    let event = match event {
                        FollowEvent::Unreachable { reason } => {
                            DownloadEvent::OwnerUnreachable { reason }
                        }
                        FollowEvent::Reachable => DownloadEvent::OwnerReachable,
                    };
                    cancelled |= on_event(event).is_break();
                })
                .await?;
                if cancelled {
                    return Err(DownloadError::Cancelled);
                }
            }
            // Refused data: most likely the owner published a new version
            // meanwhile and stopped serving this one's blobs. The next
            // connection finds out, and continues only if so.
            Err(
                error @ DownloadError::Transport(
                    TransportError::Refused | TransportError::OwnerFailed,
                ),
            ) if new_versions < MAX_NEW_VERSIONS && carried.refused.is_none() => {
                new_versions += 1;
                carried.refused = Some((carried.seq, error));
            }
            // Long on the relay: a new connection looks for a direct path,
            // and the download goes on where it was.
            Err(DownloadError::LookAgain) => {}
            // The connection was lost (a network change, a timeout): a new
            // one goes on where this one was. An owner that cannot be
            // reached fails the connect, which says so.
            Err(DownloadError::Transport(TransportError::Interrupted(reason)))
                if carried.lost < MAX_LOST_IN_A_ROW
                    || carried.received > carried.received_at_loss =>
            {
                if carried.received > carried.received_at_loss {
                    carried.lost = 0;
                }
                carried.lost += 1;
                carried.received_at_loss = carried.received;
                if on_event(DownloadEvent::ConnectionLost { reason: &reason }).is_break() {
                    return Err(DownloadError::Cancelled);
                }
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(carried.lost))).await;
            }
            // The owner is still preparing a file (ADR 0007): the next
            // connection fetches the record again and follows a newer
            // version. Never given up on.
            Err(DownloadError::Transport(TransportError::NotReady)) => {
                tokio::time::sleep(not_ready::FIRST_PAUSE).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_over(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    joined: &JoinedStick,
    floor: Option<ManifestVersion>,
    output_dir: &Path,
    policy: &RelayPolicy,
    carried: &mut Carried,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<Over, DownloadError> {
    // What the user was told. The tracker below starts from this, not from
    // the path at that later moment: hole punching often succeeds while the
    // manifest is fetched, and a change nobody reported would never be shown.
    let reported_path = connection.path();
    let connected_usage = connection.usage();
    if on_event(DownloadEvent::Connected {
        path: reported_path,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    let (Listing { version, manifest }, cipher) =
        fetch_listing(fetcher, connection, &store, ticket, floor).await?;
    joined::remember_name(&store, joined, &manifest.name).await?;
    if let Some((refused_seq, error)) = carried.refused.take() {
        if refused_seq.is_some_and(|refused| version.seq <= refused) {
            // Still the same version: the refusal was real.
            return Err(error);
        }
        if refused_seq.is_some()
            && on_event(DownloadEvent::NewVersion { seq: version.seq }).is_break()
        {
            return Err(DownloadError::Cancelled);
        }
    }
    carried.seq = Some(version.seq);
    // What this folder already holds, not what this device has seen.
    let unchanged = joined.completed == Some(version);
    if on_event(DownloadEvent::Manifest {
        seq: version.seq,
        files: manifest.entries.len(),
        bytes: manifest.total_size(),
        unchanged,
        partial: manifest.partial,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    let top = listing::top_entries(&manifest);
    if on_event(DownloadEvent::Contents {
        entries: &top,
        manifest: &manifest,
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    // What the folder already holds, file by file. Nothing is fetched for a
    // file that is there, and nothing is written over a file changed there.
    let plan = joined::plan_round(&store, ticket, output_dir, &manifest, &cipher).await?;
    joined::take_plan(&store, ticket, &plan, &manifest, on_event).await?;
    if plan.no_longer_in_stick > 0
        && on_event(DownloadEvent::NoLongerInStick {
            count: plan.no_longer_in_stick,
        })
        .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    // ADR 0006: never wait, never ask; data through a relay is only slowed.
    let mut limiter =
        RelayLimiter::new(policy, connection.usage().relay, std::time::Instant::now());

    let partial = {
        let output_dir = output_dir.to_path_buf();
        let marked = (joined.kind == StickKind::App).then_some(joined.stick_id);
        run_blocking(move || receive::partial_for(&output_dir, marked)).await?
    };
    let mut receiving = ReceivingPath::new(reported_path, connected_usage, &mut carried.watch);
    let FileCounts {
        already_here,
        changed_here,
        left_out,
        not_chosen,
    } = fetch_files(
        fetcher,
        connection,
        &store,
        ticket,
        &cipher,
        &manifest,
        &plan.files,
        &plan.saved,
        output_dir,
        &partial,
        &mut receiving,
        &mut limiter,
        &mut carried.received,
        on_event,
    )
    .await?;
    tokio::fs::remove_dir_all(&partial)
        .await
        .map_err(io_error(&partial))?;
    if manifest.partial {
        return Ok(Over::Partial { seq: version.seq });
    }
    // Complete only if the folder holds every file of this version.
    if changed_here == 0 && left_out == 0 {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        run_blocking(move || store.complete_download(&stick_id, version)).await?;
    }

    Ok(Over::Done(DownloadReport {
        seq: version.seq,
        files: manifest.entries.len(),
        bytes: manifest.total_size(),
        // Both are counted over all connections by `download`.
        received: 0,
        already_here,
        changed_here,
        saved_as: plan.renamed.iter().map(|renamed| renamed.files).sum(),
        left_out,
        not_chosen,
        usage: PathUsage::default(),
    }))
}

#[cfg(test)]
mod tests;
