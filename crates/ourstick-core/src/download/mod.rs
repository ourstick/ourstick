//! The reader's side: fetch and check the latest record, fetch and decrypt the
//! manifest, then fetch, decrypt and write every file. Interrupted downloads
//! resume, because the fetcher keeps partial blobs.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

mod entry;
mod files;
mod folder;
mod follow;
mod listing;
mod path;
mod relay;
mod types;

use entry::download_file;
use files::prepare_partial_dir;
use folder::{plan_folder, FilePlan};
use listing::fetch_listing;
use path::ReceivingPath;
use relay::{settle, RelayState, Watch};

pub use follow::{wait_for_newer, FollowEvent, FOLLOW_EVERY};
pub use listing::{list, Listing};
pub use relay::{RelayConsent, RelayPolicy, RelayQuestion};
pub use types::{DownloadError, DownloadEvent, DownloadReport};

/// How many times a download reconnects (budget used up, direct path lost)
/// before giving up.
const MAX_RECONNECTS: u32 = 3;
/// How many new versions a download follows before giving up; a folder that
/// never stops changing cannot keep it going forever.
const MAX_NEW_VERSIONS: u32 = 10;

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
}

use crate::blocking::run_blocking;
use crate::clock::Clock;
use crate::store::{DownloadedFile, JoinedStick, ManifestVersion, Store, StoreError};
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
/// Whether file data may go through the relay follows `policy` (ADR 0006);
/// `consent` is asked when a larger transfer has no direct path.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    ticket: &Ticket,
    output_dir: &Path,
    policy: &RelayPolicy,
    consent: &impl RelayConsent,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
    let stick_id = ticket.stick_id;
    {
        let store = store.clone();
        let output_dir = output_dir.to_path_buf();
        let now = clock.now_ms();
        run_blocking(move || match store.joined_stick(&stick_id)? {
            Some(joined) if joined.output_dir == output_dir => Ok(()),
            // Another folder holds nothing yet: it replaces the old one.
            Some(_) => store.move_joined_stick(&stick_id, &output_dir),
            None => store.add_joined_stick(&JoinedStick::new(stick_id, output_dir, now)),
        })
        .await?;
    }

    let mut relay = RelayState::default();
    let mut reconnects = 0;
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
            consent,
            &mut relay,
            &mut carried,
            on_event,
        )
        .await;
        let usage = connection.usage();
        total.direct += usage.direct;
        total.relay += usage.relay;
        total.direct_ipv6 += usage.direct_ipv6;
        relay.relay_used = total.relay;
        connection.close().await;
        let event = match result {
            Ok(report) => {
                return Ok(DownloadReport {
                    received: carried.received,
                    usage: total,
                    ..report
                })
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
                continue;
            }
            Err(DownloadError::RelayBudgetUsed) if reconnects < MAX_RECONNECTS => {
                relay.budget_spent = true;
                DownloadEvent::RelayBudgetUsed {
                    bytes: relay.relay_used,
                }
            }
            Err(DownloadError::DirectLost) if reconnects < MAX_RECONNECTS => {
                relay.direct_was_lost = true;
                DownloadEvent::DirectLost
            }
            Err(error) => return Err(error),
        };
        reconnects += 1;
        if on_event(event).is_break() {
            return Err(DownloadError::Cancelled);
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
    consent: &impl RelayConsent,
    relay: &mut RelayState,
    carried: &mut Carried,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
) -> Result<DownloadReport, DownloadError> {
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
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    // What the folder already holds, file by file. Nothing is fetched for a
    // file that is there, and nothing is written over a file changed there.
    let plan = {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        let recorded = run_blocking(move || store.downloaded_files(&stick_id)).await?;
        let output_dir = output_dir.to_path_buf();
        let manifest = manifest.clone();
        let cipher = cipher.clone();
        run_blocking(move || plan_folder(&output_dir, &manifest, &recorded, cipher.as_ref()))
            .await?
    };
    for found in plan.found.clone() {
        record_downloaded(&store, ticket, found).await?;
    }
    if plan.no_longer_in_stick > 0
        && on_event(DownloadEvent::NoLongerInStick {
            count: plan.no_longer_in_stick,
        })
        .is_break()
    {
        return Err(DownloadError::Cancelled);
    }

    // ADR 0006: a short wait for a direct path, then a relay budget, then only
    // a direct path unless the user agrees. The record and manifest above may
    // use the relay. Nothing to fetch, nothing to wait for.
    let mut remaining = 0;
    for (entry, file) in manifest.entries.iter().zip(&plan.files) {
        if matches!(file, FilePlan::Download { .. }) {
            remaining += entry
                .size
                .saturating_sub(fetcher.local_bytes(&entry.hash).await?);
        }
    }
    let to_fetch = plan
        .files
        .iter()
        .any(|file| matches!(file, FilePlan::Download { .. }));
    let watch = if to_fetch {
        settle(
            fetcher, connection, ticket, policy, consent, relay, remaining, on_event,
        )
        .await?
    } else {
        Watch::Nothing
    };

    let partial = {
        let output_dir = output_dir.to_path_buf();
        run_blocking(move || prepare_partial_dir(&output_dir)).await?
    };
    let mut receiving = ReceivingPath::new(reported_path, connected_usage);
    let (mut already_here, mut changed_here) = (0, 0);
    for (entry, file) in manifest.entries.iter().zip(&plan.files) {
        let event = match *file {
            FilePlan::AlreadyHere => {
                already_here += 1;
                DownloadEvent::FileAlreadyHere {
                    path: &entry.path,
                    size: entry.size,
                }
            }
            FilePlan::ChangedHere => {
                changed_here += 1;
                DownloadEvent::FileChangedHere {
                    path: &entry.path,
                    size: entry.size,
                }
            }
            FilePlan::Download { replaceable } => {
                let written = download_file(
                    fetcher,
                    connection,
                    &cipher,
                    entry,
                    output_dir,
                    &partial,
                    &mut receiving,
                    watch,
                    replaceable,
                    &mut carried.received,
                    on_event,
                )
                .await?;
                match written {
                    Some(stamp) => {
                        let file = DownloadedFile {
                            path: entry.path.clone(),
                            stamp,
                            blob_hash: entry.hash,
                        };
                        record_downloaded(&store, ticket, file).await?;
                    }
                    None => changed_here += 1,
                }
                continue;
            }
        };
        if on_event(event).is_break() {
            return Err(DownloadError::Cancelled);
        }
    }
    tokio::fs::remove_dir_all(&partial)
        .await
        .map_err(io_error(&partial))?;
    // Complete only if the folder holds every file of this version.
    if changed_here == 0 {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        run_blocking(move || store.complete_download(&stick_id, version)).await?;
    }

    Ok(DownloadReport {
        seq: version.seq,
        files: manifest.entries.len(),
        bytes: manifest.total_size(),
        // Both are counted over all connections by `download`.
        received: 0,
        already_here,
        changed_here,
        usage: PathUsage::default(),
    })
}

async fn record_downloaded(
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    file: DownloadedFile,
) -> Result<(), DownloadError> {
    let store = store.clone();
    let stick_id = ticket.stick_id;
    run_blocking(move || store.record_downloaded_file(&stick_id, &file)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_maps_to_its_own_error() {
        assert!(matches!(
            cancelled(TransportError::Cancelled),
            DownloadError::Cancelled
        ));
        assert!(matches!(
            cancelled(TransportError::Corrupt),
            DownloadError::Transport(TransportError::Corrupt)
        ));
    }
}
