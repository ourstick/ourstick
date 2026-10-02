//! The space ledger (ADR 0008 §1, as amended): how much room is left on a
//! disk once the system's reserve, every sharer's cache budget and every
//! download's bytes still to come are counted, across all the OurStick
//! programs of this device. Claims live in the store and count only while
//! their holder's lock is held ([`crate::locks::is_held`]), so a program that
//! ended or crashed holds nothing. Blocking; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::locks::{is_held, LockError};
use crate::share::{MAX_BUDGET, MIN_BUDGET};
use crate::stick_id::StickId;
use crate::store::{ClaimKind, SpaceClaim, Store, StoreError};

/// The system's reserve is this share of a disk…
const RESERVE_SHARE: u64 = 10;
/// …at most this.
pub const MAX_RESERVE: u64 = 20 << 30;
/// The device's cache budget is this share of the free space (ADR 0007).
const BUDGET_SHARE: u64 = 10;

/// Why room could not be had. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum SpaceError {
    #[error("does not fit: needs {needed} bytes, {available} are left")]
    DoesNotFit { needed: u64, available: u64 },
    #[error("not enough room for another share's working space ({free} bytes free on the cache's disk; other shares hold {held}, downloads {downloads}); stop one of them first")]
    NoRoomForCache {
        free: u64,
        held: u64,
        downloads: u64,
    },
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Lock(#[from] LockError),
}

/// What the system says about disks; a trait so tests need no real disk.
pub trait Disks: Send + Sync {
    /// Which disk `path` is on (the nearest existing folder above it if it
    /// is not there yet): two paths on one disk give the same key.
    fn key(&self, path: &Path) -> std::io::Result<String>;
    /// Bytes free for this user on the disk of `path`.
    fn free(&self, path: &Path) -> std::io::Result<u64>;
    /// The disk's whole size.
    fn total(&self, path: &Path) -> std::io::Result<u64>;
}

/// The real disks.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDisks;

/// `path`, or the nearest folder above it that exists.
fn existing(path: &Path) -> &Path {
    path.ancestors()
        .find(|ancestor| ancestor.exists())
        .unwrap_or(path)
}

impl Disks for SystemDisks {
    fn key(&self, path: &Path) -> std::io::Result<String> {
        disk_key(existing(path))
    }

    fn free(&self, path: &Path) -> std::io::Result<u64> {
        fs4::available_space(existing(path))
    }

    fn total(&self, path: &Path) -> std::io::Result<u64> {
        fs4::total_space(existing(path))
    }
}

#[cfg(unix)]
fn disk_key(path: &Path) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    Ok(format!("dev:{}", std::fs::metadata(path)?.dev()))
}

/// The volume of the path with links resolved (so a `subst` letter counts
/// as the disk it points to), `\\?\C:` and `C:` alike.
#[cfg(not(unix))]
fn disk_key(path: &Path) -> std::io::Result<String> {
    use std::path::{Component, Prefix};
    let resolved = std::fs::canonicalize(path)?;
    let volume = match resolved.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                format!("{}:", (letter as char).to_ascii_uppercase())
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                r"\\{}\{}",
                server.to_string_lossy().to_lowercase(),
                share.to_string_lossy().to_lowercase()
            ),
            _ => prefix.as_os_str().to_string_lossy().to_lowercase(),
        },
        _ => resolved.to_string_lossy().into_owned(),
    };
    Ok(format!("vol:{volume}"))
}

/// The room the system keeps for itself on a disk of `total` bytes.
pub fn reserve(total: u64) -> u64 {
    (total / RESERVE_SHARE).min(MAX_RESERVE)
}

/// What is left on a disk: free − reserve − what live claims hold there.
pub fn room_left(free: u64, total: u64, held: u64) -> u64 {
    free.saturating_sub(reserve(total)).saturating_sub(held)
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> SpaceError + '_ {
    move |source| SpaceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The live claims among `all`, but `except`'s own of those kinds.
fn live<'a>(
    data_dir: &Path,
    all: &'a [SpaceClaim],
    except: Option<(StickId, &[ClaimKind])>,
) -> Result<Vec<&'a SpaceClaim>, LockError> {
    let mut live = Vec::new();
    for claim in all {
        if except.is_some_and(|(stick_id, kinds)| {
            stick_id == claim.stick_id && kinds.contains(&claim.kind)
        }) {
            continue;
        }
        if is_held(data_dir, claim.kind, &claim.stick_id)? {
            live.push(claim);
        }
    }
    Ok(live)
}

fn held_on(claims: &[&SpaceClaim], disk: &str) -> u64 {
    claims
        .iter()
        .filter(|claim| claim.disk == disk)
        .fold(0u64, |held, claim| held.saturating_add(claim.bytes))
}

/// What is left on the disk of `path`, every live claim counted but
/// `except`'s own (e.g. a download asking how much more it may take).
pub fn available(
    disks: &dyn Disks,
    store: &dyn Store,
    data_dir: &Path,
    path: &Path,
    except: Option<(StickId, ClaimKind)>,
) -> Result<u64, SpaceError> {
    let disk = disks.key(path).map_err(io(path))?;
    let free = disks.free(path).map_err(io(path))?;
    let total = disks.total(path).map_err(io(path))?;
    let all = store.space_claims()?;
    let except = except.map(|(stick_id, kind)| (stick_id, vec![kind]));
    let live = live(
        data_dir,
        &all,
        except
            .as_ref()
            .map(|(stick_id, kinds)| (*stick_id, kinds.as_slice())),
    )?;
    Ok(room_left(free, total, held_on(&live, &disk)))
}

/// The cache budget of the stick `stick_id` about to be shared, whose lock
/// this program holds: one budget for the whole device (10 % of the cache
/// disk's free space plus what other shares hold, 2 to 40 GB; the room that
/// running downloads hold there is not free), split
/// equally between the `together` sticks started at once (this one
/// included; 1 for one): this sharer gets its part, or what the others
/// leave if that is less, but at least 2 GB while that much is free.
/// Recorded as a claim until the stick's lock goes. Running sharers keep
/// theirs.
pub fn claim_cache(
    disks: &dyn Disks,
    store: &dyn Store,
    data_dir: &Path,
    stick_id: &StickId,
    cache_dir: &Path,
    together: usize,
) -> Result<u64, SpaceError> {
    let disk = disks.key(cache_dir).map_err(io(cache_dir))?;
    let free = disks.free(cache_dir).map_err(io(cache_dir))?;
    let mut outcome = None;
    store.claim_space(&mut |all| {
        let decided = live(data_dir, all, Some((*stick_id, &[ClaimKind::Cache]))).map(|live| {
            let sum = |of: &dyn Fn(ClaimKind) -> bool| {
                live.iter()
                    .filter(|claim| claim.disk == disk && of(claim.kind))
                    .fold(0u64, |held, claim| held.saturating_add(claim.bytes))
            };
            let held = sum(&|kind| kind == ClaimKind::Cache);
            // Promised to downloads: not free for a cache.
            let downloads = sum(&|kind| kind != ClaimKind::Cache);
            cache_share(free.saturating_sub(downloads), held, together).map_err(|error| match error
            {
                // Say what took the room: the disk's free bytes, and the
                // downloads' part of them.
                SpaceError::NoRoomForCache { held, .. } => SpaceError::NoRoomForCache {
                    free,
                    held,
                    downloads,
                },
                other => other,
            })
        });
        let claim = match &decided {
            Ok(Ok(bytes)) => vec![SpaceClaim {
                stick_id: *stick_id,
                kind: ClaimKind::Cache,
                disk: disk.clone(),
                bytes: *bytes,
            }],
            _ => Vec::new(),
        };
        outcome = Some(decided);
        claim
    })?;
    outcome.expect("decided")?
}

/// This sharer's part of the device's cache budget, when `free` bytes are
/// free on the cache's disk, other live shares hold `held`, and `together`
/// sticks start at once.
pub fn cache_share(free: u64, held: u64, together: usize) -> Result<u64, SpaceError> {
    let total = (free.saturating_add(held) / BUDGET_SHARE).clamp(MIN_BUDGET, MAX_BUDGET);
    let part = total / u64::try_from(together.max(1)).unwrap_or(u64::MAX);
    let mine = part.min(total.saturating_sub(held)).max(MIN_BUDGET);
    if free < mine {
        return Err(SpaceError::NoRoomForCache {
            free,
            held,
            downloads: 0,
        });
    }
    Ok(mine)
}

/// Claims room for a download of `stick_id` into `out_dir` that still
/// needs `needed` bytes there (a `Download` claim), and `scratch` bytes on
/// the disk of `data_dir` while it runs (its batches; a `Scratch` claim),
/// both in one transaction. Refused with [`SpaceError::DoesNotFit`] if
/// either disk has not that much left. This program must hold the stick's
/// download lock. Update it as files land ([`Store::update_space_claim`]);
/// it ends with the lock, or when released (`DownloadRoom`).
pub fn claim_download(
    disks: &dyn Disks,
    store: &dyn Store,
    data_dir: &Path,
    stick_id: &StickId,
    out_dir: &Path,
    needed: u64,
    scratch: u64,
) -> Result<(), SpaceError> {
    let out_disk = disks.key(out_dir).map_err(io(out_dir))?;
    let data_disk = disks.key(data_dir).map_err(io(data_dir))?;
    let claims = [
        SpaceClaim {
            stick_id: *stick_id,
            kind: ClaimKind::Download,
            disk: out_disk.clone(),
            bytes: needed,
        },
        SpaceClaim {
            stick_id: *stick_id,
            kind: ClaimKind::Scratch,
            disk: data_disk.clone(),
            bytes: scratch,
        },
    ];
    let mut wants = vec![(out_disk.clone(), out_dir, needed)];
    if data_disk == out_disk {
        wants[0].2 += scratch;
    } else {
        wants.push((data_disk, data_dir, scratch));
    }
    let mut rooms = Vec::new();
    for (disk, path, want) in &wants {
        let free = disks.free(path).map_err(io(path))?;
        let total = disks.total(path).map_err(io(path))?;
        rooms.push((disk.clone(), free, total, *want));
    }
    let mut outcome = None;
    store.claim_space(&mut |all| {
        let own = [ClaimKind::Download, ClaimKind::Scratch];
        let checked = live(data_dir, all, Some((*stick_id, &own))).map(|live| {
            for (disk, free, total, want) in &rooms {
                let available = room_left(*free, *total, held_on(&live, disk));
                if *want > available {
                    return Err(SpaceError::DoesNotFit {
                        needed: *want,
                        available,
                    });
                }
            }
            Ok(())
        });
        let recorded = if matches!(checked, Ok(Ok(()))) {
            claims.to_vec()
        } else {
            Vec::new()
        };
        outcome = Some(checked);
        recorded
    })?;
    outcome.expect("decided")?
}

mod room;

pub use room::DownloadRoom;

#[cfg(test)]
mod tests;
