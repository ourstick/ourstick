//! The space ledger (ADR 0008 §1, as amended): how much room is left on a
//! disk once the system's reserve, every sharer's cache budget and every
//! download's bytes still to come are counted, across all the OurStick
//! programs of this device. Claims live in the store and count only while
//! their holder's lock is held ([`crate::locks::is_held`]), so a program that
//! ended or crashed holds nothing. Blocking; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::locks::{is_held, LockError};
use crate::stick_id::StickId;
use crate::store::{ClaimKind, SpaceClaim, Store, StoreError};

/// The system's reserve is this share of a disk (a twentieth)…
const RESERVE_SHARE: u64 = 20;
/// …at most this (ADR 0008, amendment of 2026-11-06). It was a tenth, at
/// most 20 GiB: a Mac of 228 GB with 13 GB free then took no download at
/// all. A copy in Finder or Explorer keeps nothing back; Android warns of
/// low storage under 5 %, at most 500 MB, and 2 GiB stays above that.
pub const MAX_RESERVE: u64 = 2 << 30;

/// Why room could not be had. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum SpaceError {
    #[error("does not fit: needs {needed} bytes, {available} are left")]
    DoesNotFit { needed: u64, available: u64 },
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

/// How far apart two looks at one pool's free space may be: it moves
/// between them.
const SAME_POOL_SLACK: u64 = 64 << 20;

/// Whether two disk keys with these (free, total) figures are one pool of
/// room: a phone's shared storage (a FUSE mount over `/data`), btrfs
/// subvolumes, the APFS volumes of one container each have a device of
/// their own, while every byte written to one is gone from the others.
/// Two real disks of one size and nearly the same free space count as one
/// too, which only ever leaves less room.
fn same_pool((free_a, total_a): (u64, u64), (free_b, total_b): (u64, u64)) -> bool {
    total_a == total_b && free_a.abs_diff(free_b) <= SAME_POOL_SLACK
}

/// The free space and size of the disk of `path`.
fn figures(disks: &dyn Disks, path: &Path) -> Result<(u64, u64), SpaceError> {
    Ok((
        disks.free(path).map_err(io(path))?,
        disks.total(path).map_err(io(path))?,
    ))
}

/// What live claims hold on any of `keys` (one pool).
fn held_on_any(claims: &[&SpaceClaim], keys: &[String]) -> u64 {
    keys.iter()
        .fold(0u64, |held, key| held.saturating_add(held_on(claims, key)))
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
    let mut keys = vec![disks.key(path).map_err(io(path))?];
    let (free, total) = figures(disks, path)?;
    // The data folder's disk too, if it is the same pool: its downloads'
    // scratch takes from this room.
    if let (Ok(data_disk), Ok(data)) = (disks.key(data_dir), figures(disks, data_dir)) {
        if data_disk != keys[0] && same_pool((free, total), data) {
            keys.push(data_disk);
        }
    }
    let all = store.space_claims()?;
    let except = except.map(|(stick_id, kind)| (stick_id, vec![kind]));
    let live = live(
        data_dir,
        &all,
        except
            .as_ref()
            .map(|(stick_id, kinds)| (*stick_id, kinds.as_slice())),
    )?;
    Ok(room_left(free, total, held_on_any(&live, &keys)))
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
    let out = figures(disks, out_dir)?;
    let data = figures(disks, data_dir)?;
    // Each room: the disk keys whose claims take from it, its figures, and
    // what this download wants of it. One pool under two keys is one room:
    // counted apart, the bytes and the scratch would each be checked against
    // the same free space.
    let rooms = if data_disk == out_disk {
        vec![(vec![out_disk], out, needed.saturating_add(scratch))]
    } else if same_pool(out, data) {
        let free = out.0.min(data.0);
        let wants = needed.saturating_add(scratch);
        vec![(vec![out_disk, data_disk], (free, out.1), wants)]
    } else {
        vec![
            (vec![out_disk], out, needed),
            (vec![data_disk], data, scratch),
        ]
    };
    let mut outcome = None;
    store.claim_space(&mut |all| {
        let own = [ClaimKind::Download, ClaimKind::Scratch];
        let checked = live(data_dir, all, Some((*stick_id, &own))).map(|live| {
            for (keys, (free, total), want) in &rooms {
                let available = room_left(*free, *total, held_on_any(&live, keys));
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
