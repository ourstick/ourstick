//! A stick's life in the app (ADR 0009 §5): a new link, and removal, both
//! safe to stop at any point and finished at the next start. Blocking; the
//! caller has stopped sharing the stick and holds its lock (`crate::locks`).
//!
//! A new link never touches the user's files: the old id is retired, and
//! what is left of it is its secrets and cache. A removal deletes a stick's
//! copied files only behind its drive marker, never across a mount point,
//! never a linked folder, never the device's node key (`CLAUDE.md`, rule 9).

use std::path::Path;

use crate::cipher::StickKey;
use crate::clock::{Clock, SystemClock};
use crate::device::{random_key, StickKeys};
use crate::secrets::{SecretError, SecretName, SecretStore};
use crate::share::{drive_present, mark_drive, ShareError};
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{OwnedStick, StickKind, Store, StoreError};
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

/// The folder of a stick's suitcases (ADR 0007).
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
            &signing_key.to_bytes(),
        )?;
        secrets.put(&SecretName::StickKey(new_id), stick_key.as_bytes())?;
        secrets.put(&SecretName::StickNodeKey(new_id), &node_key)
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
    // again at the next start if it fails now.
    if let Err(error) = remark_drive(&new, stick_id) {
        tracing::warn!(%error, "the stick's marker names its old link until the next start");
    } else if let Err(error) = clean_up_retired(store, secrets, data_dir) {
        tracing::warn!(%error, "the old link's keys are deleted at the next start");
    }
    Ok(StickKeys {
        stick: new,
        signing_key,
        stick_key,
        node_key,
    })
}

/// An app stick's marker names `old`: it names the stick now.
fn remark_drive(stick: &OwnedStick, old: &StickId) -> Result<(), LifeError> {
    if stick.kind == StickKind::App && drive_present(&stick.root_path, old) {
        mark_drive(&stick.root_path, &stick.stick_id)?;
    }
    Ok(())
}

/// At start, before anything is shared or mounted: an app stick whose marker
/// still names a retired id (a new link stopped halfway) is marked anew, and
/// what is left of retired ids is deleted. One that fails (its drive is not
/// there, say) is kept for the next start and does not stop the others;
/// the failures are returned, by stick name.
pub fn finish_new_links(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<Vec<(String, LifeError)>, LifeError> {
    let sticks = store.owned_sticks()?;
    let mut failed = Vec::new();
    for old in store.retired_sticks()? {
        let mut remarked = true;
        for stick in &sticks {
            if let Err(error) = remark_drive(stick, &old) {
                failed.push((stick.name.clone(), error));
                remarked = false;
            }
        }
        // Forgotten only once its stick's marker names the new id: else the
        // next start could no longer tell which marker to fix.
        if remarked {
            if let Err(error) = clean_up(store, secrets, data_dir, &old) {
                failed.push((String::new(), error));
            }
        }
    }
    Ok(failed)
}

fn clean_up_retired(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<(), LifeError> {
    for old in store.retired_sticks()? {
        clean_up(store, secrets, data_dir, &old)?;
    }
    Ok(())
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
    if cfg!(target_os = "macos") && stick.kind == StickKind::App && stick.image_path.is_none() {
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

/// At start, before anything is shared or mounted: every removal stopped
/// halfway is finished; one that fails again stays, and says why.
pub fn finish_removals(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    drive: &dyn DriveOps,
) -> Result<Vec<(String, LifeError)>, LifeError> {
    let mut failed = Vec::new();
    for stick in store.removing_sticks()? {
        if let Err(error) = finish_removal(store, secrets, data_dir, &stick, drive) {
            failed.push((stick.name.clone(), error));
        }
    }
    Ok(failed)
}

fn finish_removal(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick: &OwnedStick,
    drive: &dyn DriveOps,
) -> Result<(), LifeError> {
    image_known(stick)?;
    // Copied files: only an app stick's, only behind its marker; a disk
    // image's go with the image.
    if stick.kind == StickKind::App
        && stick.image_path.is_none()
        && drive_present(&stick.root_path, &stick.stick_id)
    {
        empty::empty_drive(&stick.root_path, data_dir)?;
    }
    drive.take_away(stick).map_err(LifeError::Drive)?;
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

mod empty;

#[cfg(test)]
mod tests;
