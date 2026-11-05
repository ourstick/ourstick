//! A stick's life in the app (ADR 0009 §5): a new link, and removal, both
//! safe to stop at any point and finished at the next start. Blocking; the
//! caller has stopped sharing the stick and holds its lock (`crate::locks`).
//!
//! A new link never touches the user's files: the old id is retired, and
//! what is left of it is its secrets and cache. Neither does a removal (ADR
//! 0028 §7): an older app stick's folder stays with its files, only its
//! drive and marker go; a newer one's empty folder goes. Never a linked
//! folder, never the device's node key (`CLAUDE.md`, rule 9).

use std::path::Path;

use crate::cipher::StickKey;
use crate::clock::{Clock, SystemClock};
use crate::device::{random_key, StickKeys};
use crate::manifest::DRIVE_MARKER;
use crate::secrets::{SecretError, SecretName, SecretStore};
use crate::share::{drive_present, mark_drive, ShareError};
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{OwnedStick, RetiredStick, StickKind, Store, StoreError};
use crate::transport::iroh::IrohConfig;

/// Why a step of a stick's life failed. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum LifeError {
    #[error("the stick is not on this device")]
    NotFound,
    #[error("{0}; the stick stays as it is and is finished at the next start")]
    Drive(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Share(#[from] ShareError),
    #[error(transparent)]
    Lock(#[from] crate::locks::LockError),
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What only the app knows how to undo for a stick being removed: its
/// bookmark, drive letter or disk image (detached and deleted). An error
/// leaves the stick being removed, to be finished at the next start.
pub trait DriveOps {
    fn take_away(&self, stick: &OwnedStick) -> Result<(), String>;
}

/// No drive to take away (the command line's sticks, tests).
pub struct NoDrive;

impl DriveOps for NoDrive {
    fn take_away(&self, _stick: &OwnedStick) -> Result<(), String> {
        Ok(())
    }
}

/// The folder of a stick's cache: its latest manifest and outboards (ADR
/// 0017).
fn cache_dir(data_dir: &Path, stick_id: &StickId) -> std::path::PathBuf {
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    data_dir.join("cache").join(hex)
}

/// Deletes a stick's own secrets; the device's node key is never one of
/// them. Missing ones are fine (a step done before).
fn delete_secrets(secrets: &dyn SecretStore, stick_id: &StickId) -> Result<(), LifeError> {
    for name in [
        SecretName::StickSigningKey(*stick_id),
        SecretName::StickKey(*stick_id),
        SecretName::StickNodeKey(*stick_id),
    ] {
        secrets.delete(&name)?;
    }
    Ok(())
}

fn remove_cache(data_dir: &Path, stick_id: &StickId) -> Result<(), LifeError> {
    let dir = cache_dir(data_dir, stick_id);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LifeError::Io { path: dir, source }),
    }
}

/// "Give it a new link": new keys (a node key of its own too), stored first;
/// then, in one transaction, the new id takes the old one's place, folder,
/// settings and links, and the old id is retired; then the drive marker
/// names the new id; then the old id's secrets and cache go. Everyone with
/// the old link loses it. Returns the stick with its new keys.
pub fn give_new_link(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick_id: &StickId,
) -> Result<StickKeys, LifeError> {
    let old = store.owned_stick(stick_id)?.ok_or(LifeError::NotFound)?;
    let signing_key = StickSigningKey::generate();
    let stick_key = StickKey::generate();
    let node_key = random_key();
    let new_id = signing_key.stick_id();
    let stored = (|| {
        secrets.put(
            &SecretName::StickSigningKey(new_id),
            zeroize::Zeroizing::new(signing_key.to_bytes()).as_slice(),
        )?;
        secrets.put(&SecretName::StickKey(new_id), stick_key.as_bytes())?;
        secrets.put(&SecretName::StickNodeKey(new_id), node_key.as_slice())
    })();
    let new = OwnedStick {
        stick_id: new_id,
        node_id: Some(IrohConfig::node_id_of(&node_key)),
        last_signed_seq: 0,
        latest_record: None,
        latest_partial: false,
        ..old.clone()
    };
    let replaced = stored
        .map_err(LifeError::from)
        .and_then(|()| Ok(store.replace_owned_stick(stick_id, &new, SystemClock.now_ms())?));
    if let Err(error) = replaced {
        // Nothing changed: the new keys belong to no stick, so they go.
        if let Err(left) = delete_secrets(secrets, &new_id) {
            tracing::warn!(error = %left, "the keys of a failed new link could not be deleted");
        }
        return Err(error);
    }
    // The new link is made from here on: what is left is tidying up, done
    // again at the next start (or mount) if it cannot be done now.
    let settled = store
        .retired_sticks()
        .map_err(LifeError::from)
        .and_then(|retired| {
            let sticks = store.owned_sticks()?;
            settle_marker(&new, &retired, &sticks)
        });
    match settled {
        Ok(true) => {
            if let Err(error) = clean_up(store, secrets, data_dir, stick_id) {
                tracing::warn!(%error, "the old link's keys are deleted at the next start");
            }
        }
        Ok(false) => {} // Its drive is not there: at the next start or mount.
        Err(error) => {
            tracing::warn!(%error, "the stick's marker names its old link until the next start");
        }
    }
    Ok(StickKeys {
        stick: new,
        signing_key,
        stick_key,
        node_key,
    })
}

/// Whether `stick`'s drive marker names it, after rewriting a marker that
/// names one of its retired ids (a new link, or several, stopped before the
/// marker was written). False while its drive is not there (unplugged, or a
/// macOS image not attached): nothing can be known until it is back.
fn settle_marker(
    stick: &OwnedStick,
    retired: &[RetiredStick],
    sticks: &[OwnedStick],
) -> Result<bool, LifeError> {
    if !stick.is_drive() || drive_present(&stick.root_path, &stick.stick_id) {
        return Ok(true);
    }
    // Only this stick's own earlier ids: a marker naming another stick's
    // (its drive mounted here, say) is that drive's, never claimed.
    for entry in retired {
        let own = successor(&entry.old, retired, sticks)
            .is_some_and(|successor| successor.stick_id == stick.stick_id);
        if own && drive_present(&stick.root_path, &entry.old) {
            mark_drive(&stick.root_path, &stick.stick_id)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// The live stick that took `old`'s place, following new links given one
/// after another; `None` if it was removed since.
fn successor(old: &StickId, retired: &[RetiredStick], sticks: &[OwnedStick]) -> Option<OwnedStick> {
    let mut id = *old;
    // Each step goes to a newer id; never more steps than retired ids.
    for _ in 0..=retired.len() {
        let next = retired.iter().find(|entry| entry.old == id)?.replaced_by;
        if let Some(stick) = sticks.iter().find(|stick| stick.stick_id == next) {
            return Some(stick.clone());
        }
        id = next;
    }
    None
}

/// At start, before anything is shared, and whenever a drive is mounted
/// (while no new link is being given for one of its sticks):
/// a stick whose marker still names a retired id is marked anew, and a
/// retired id whose stick's marker names the stick now is forgotten with
/// its secrets and cache. One whose drive is not there is kept for later;
/// one that fails (a marker that cannot be written) is kept and returned,
/// by stick name, and does not stop the others.
pub fn finish_new_links(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<Vec<(String, LifeError)>, LifeError> {
    // Retired ids first: a new link committed between the two reads then
    // has its new stick among `sticks`, never an old id without a successor
    // (which would read as "removed" and lose the old id too early).
    let retired = store.retired_sticks()?;
    let sticks = store.owned_sticks()?;
    let mut failed = Vec::new();
    let mut settled: Vec<StickId> = Vec::new();
    for stick in &sticks {
        match settle_marker(stick, &retired, &sticks) {
            Ok(true) => settled.push(stick.stick_id),
            Ok(false) => {}
            Err(error) => failed.push((stick.name.clone(), error)),
        }
    }
    for entry in &retired {
        let done = match successor(&entry.old, &retired, &sticks) {
            Some(stick) => settled.contains(&stick.stick_id),
            // Removed since: nothing of the old id is needed any more.
            None => true,
        };
        if done {
            if let Err(error) = clean_up(store, secrets, data_dir, &entry.old) {
                let name = successor(&entry.old, &retired, &sticks).map_or_else(
                    || "a removed stick's old link".to_owned(),
                    |stick| format!("{}'s old link", stick.name),
                );
                failed.push((name, error));
            }
        }
    }
    Ok(failed)
}

/// Deletes what is left of a retired id.
fn clean_up(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    old: &StickId,
) -> Result<(), LifeError> {
    delete_secrets(secrets, old)?;
    remove_cache(data_dir, old)?;
    store.forget_retired(old)?;
    Ok(())
}

/// macOS: an app stick is its disk image, and the image must be known
/// before anything of it is deleted (an older stick's is recorded at the
/// app's start); its mount point alone could be another volume's.
fn image_known(stick: &OwnedStick) -> Result<(), LifeError> {
    if cfg!(target_os = "macos") && stick.is_drive() && stick.image_path.is_none() {
        return Err(LifeError::Drive(
            "the stick's disk image is not recorded yet (open OurStick, then try again)".to_owned(),
        ));
    }
    Ok(())
}

/// "Remove this stick": marked as being removed (from then on never shared
/// again), then finished ([`finish_removals`] does the same at start).
pub fn remove_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick_id: &StickId,
    drive: &dyn DriveOps,
) -> Result<(), LifeError> {
    let stick = store.owned_stick(stick_id)?.ok_or(LifeError::NotFound)?;
    // Refused before it is marked: a stick being removed is never changed
    // again, so its image could not be recorded any more.
    image_known(&stick)?;
    store.mark_removing(stick_id)?;
    let stick = store
        .removing_sticks()?
        .into_iter()
        .find(|stick| stick.stick_id == *stick_id)
        .ok_or(LifeError::NotFound)?;
    finish_removal(store, secrets, data_dir, &stick, drive)
}

/// At start, before anything is shared or mounted, and after a drive comes
/// back: every removal stopped halfway is finished; one that fails again
/// stays, and says why. A stick whose lock is held is being removed right
/// now by its own command, and is left to it.
pub fn finish_removals(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    drive: &dyn DriveOps,
) -> Result<Vec<(String, LifeError)>, LifeError> {
    let mut failed = Vec::new();
    for stick in store.removing_sticks()? {
        let _lock = match crate::locks::stick_lock(data_dir, &stick) {
            Ok(lock) => lock,
            Err(crate::locks::LockError::StickInUse { .. }) => continue,
            Err(error) => {
                failed.push((stick.name.clone(), error.into()));
                continue;
            }
        };
        if let Err(error) = finish_removal(store, secrets, data_dir, &stick, drive) {
            failed.push((stick.name.clone(), error));
        }
    }
    Ok(failed)
}

/// The empty folder of a stick with no folder of its own: deleted if it is
/// still empty (nothing writes to it); anything put there is left.
fn remove_sent_folder(root: &Path) -> Result<(), LifeError> {
    match std::fs::remove_dir(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
            tracing::warn!("a removed stick's folder was not empty; it is left");
            Ok(())
        }
        Err(source) => Err(LifeError::Io {
            path: root.to_path_buf(),
            source,
        }),
    }
}

fn finish_removal(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick: &OwnedStick,
    drive: &dyn DriveOps,
) -> Result<(), LifeError> {
    image_known(stick)?;
    if stick.is_drive() {
        // The drive first, while its marker still tells which letter is the
        // stick's (Windows); taking it away again later does nothing.
        drive.take_away(stick).map_err(LifeError::Drive)?;
        // Its files stay (ADR 0028 §7): without its marker the folder is a
        // plain folder of the user's. A disk image's marker stays in the
        // image, which the user keeps.
        if drive_present(&stick.root_path, &stick.stick_id) {
            let marker = stick.root_path.join(DRIVE_MARKER);
            match std::fs::remove_file(&marker) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(LifeError::Io {
                        path: marker,
                        source,
                    })
                }
            }
        }
    } else if stick.kind == StickKind::App {
        remove_sent_folder(&stick.root_path)?;
    }
    if stick.node_id.is_some() {
        delete_secrets(secrets, &stick.stick_id)?;
    } else {
        // An older stick: its node key is the device's, which others use.
        secrets.delete(&SecretName::StickSigningKey(stick.stick_id))?;
        secrets.delete(&SecretName::StickKey(stick.stick_id))?;
    }
    remove_cache(data_dir, &stick.stick_id)?;
    store.remove_owned_stick(&stick.stick_id)?;
    Ok(())
}

#[cfg(test)]
mod tests;
