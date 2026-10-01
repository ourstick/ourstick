//! Who may do what at the same time (ADR 0009, section 2): the operating
//! system's file locks in the data folder, released when the program ends, so
//! a leftover file is harmless. Lock files are never deleted, never read and
//! never written (on Windows a locked file cannot even be read).
//!
//! - `share-<hex stick id>.lock`: one program serves a stick, or changes it.
//! - `node-key.lock`: the device's node key, which every stick made before
//!   ADR 0009 shares: one of them is shared at a time. Taken after the
//!   stick's own lock, so the stick holding it can be found by trying the
//!   locks of those sticks.
//! - `key-file.lock`: a key file is read whole when opened and written whole
//!   on every change, so a program that uses one holds this from before it
//!   opens it until it ends. With a key file, one program at a time.
//! - `setup.lock`: held briefly while the device or a stick is made, so two
//!   programs never make the device twice or two sticks for one folder.
//!
//! All of them are tried, never waited for: no deadlock is possible, and
//! each refusal says which lock and why.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use crate::stick_id::StickId;
use crate::store::{OwnedStick, Store, StoreError};

/// Why a lock could not be taken. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("the stick \u{201c}{name}\u{201d} is already being shared on this device (by another `ourstick share` or the app)")]
    StickInUse { name: String },
    #[error("{}; sticks made before this version of OurStick share one network identity, so only one of them is shared at a time. Stop it first, or give this stick a new link.", .shared.as_ref().map_or_else(|| "another older stick is being shared".to_owned(), |name| format!("the older stick \u{201c}{name}\u{201d} is being shared")))]
    NodeKeyInUse { shared: Option<String> },
    #[error("another OurStick program is using the key file; with a key file, one program at a time (stop the other one first)")]
    KeyFileInUse,
    #[error("another OurStick program is setting up; try again in a moment")]
    SetupInUse,
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A lock this program holds, until it is dropped (or the program ends).
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

/// Takes the lock in `path`, or `None` if another program holds it.
fn try_take(path: PathBuf) -> Result<Option<Lock>, LockError> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| LockError::Io {
            path: path.clone(),
            source,
        })?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(source)) => Err(LockError::Io { path, source }),
    }
}

fn stick_lock_path(data_dir: &Path, stick_id: &StickId) -> PathBuf {
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    data_dir.join(format!("share-{hex}.lock"))
}

/// The lock of one stick: held while it is shared, or changed.
pub fn stick_lock(data_dir: &Path, stick: &OwnedStick) -> Result<Lock, LockError> {
    try_take(stick_lock_path(data_dir, &stick.stick_id))?.ok_or_else(|| LockError::StickInUse {
        name: stick.name.clone(),
    })
}

/// The lock of the device's node key, for sharing `stick`, a stick made
/// before ADR 0009 whose own lock this program holds. If another program
/// holds it, the error names the stick it shares when it can tell.
pub fn node_key_lock(
    data_dir: &Path,
    store: &dyn Store,
    stick: &OwnedStick,
) -> Result<Lock, LockError> {
    if let Some(lock) = try_take(data_dir.join("node-key.lock"))? {
        return Ok(lock);
    }
    Err(LockError::NodeKeyInUse {
        shared: older_stick_shared(data_dir, store, &stick.stick_id)?,
    })
}

/// The older stick (other than `this`) another program shares: the one whose
/// own lock is held, since a sharer of an older stick takes its stick's lock
/// before the node key's. A probe that gets a lock lets it go at once.
fn older_stick_shared(
    data_dir: &Path,
    store: &dyn Store,
    this: &StickId,
) -> Result<Option<String>, LockError> {
    for stick in store.owned_sticks()? {
        if stick.node_id.is_some() || stick.stick_id == *this {
            continue;
        }
        if try_take(stick_lock_path(data_dir, &stick.stick_id))?.is_none() {
            return Ok(Some(stick.name));
        }
    }
    Ok(None)
}

/// The key file's lock, held from before the key file is opened until the
/// program ends.
pub fn key_file_lock(data_dir: &Path) -> Result<Lock, LockError> {
    try_take(data_dir.join("key-file.lock"))?.ok_or(LockError::KeyFileInUse)
}

/// The setup lock, held briefly while the device or a stick is made.
pub fn setup_lock(data_dir: &Path) -> Result<Lock, LockError> {
    try_take(data_dir.join("setup.lock"))?.ok_or(LockError::SetupInUse)
}

/// What a program holds while it shares a stick.
#[derive(Debug)]
pub struct SharingLocks {
    _stick: Lock,
    _node_key: Option<Lock>,
}

/// Takes what sharing the stick `stick_id` needs, in order: its own lock,
/// then (for a stick made before ADR 0009) the node key's. The stick's row is
/// read again under its lock: what was read before may be stale, and a stick
/// gone meanwhile is [`StoreError::StickNotFound`], never made again.
pub fn lock_for_sharing(
    data_dir: &Path,
    store: &dyn Store,
    stick_id: &StickId,
) -> Result<(SharingLocks, OwnedStick), LockError> {
    let seen = store
        .owned_stick(stick_id)?
        .ok_or(StoreError::StickNotFound)?;
    let stick_lock = stick_lock(data_dir, &seen)?;
    let stick = store
        .owned_stick(stick_id)?
        .ok_or(StoreError::StickNotFound)?;
    let node_key = match stick.node_id {
        Some(_) => None,
        None => Some(node_key_lock(data_dir, store, &stick)?),
    };
    Ok((
        SharingLocks {
            _stick: stick_lock,
            _node_key: node_key,
        },
        stick,
    ))
}

#[cfg(test)]
mod tests;
