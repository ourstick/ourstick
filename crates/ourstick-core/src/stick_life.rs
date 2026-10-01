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
use crate::manifest::DRIVE_MARKER;
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
    secrets.put(
        &SecretName::StickSigningKey(new_id),
        &signing_key.to_bytes(),
    )?;
    secrets.put(&SecretName::StickKey(new_id), stick_key.as_bytes())?;
    secrets.put(&SecretName::StickNodeKey(new_id), &node_key)?;
    let new = OwnedStick {
        stick_id: new_id,
        node_id: Some(IrohConfig::node_id_of(&node_key)),
        last_signed_seq: 0,
        latest_record: None,
        latest_partial: false,
        ..old.clone()
    };
    store.replace_owned_stick(stick_id, &new, SystemClock.now_ms())?;
    remark_drive(&new, stick_id)?;
    clean_up_retired(store, secrets, data_dir)?;
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
/// what is left of retired ids is deleted.
pub fn finish_new_links(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<(), LifeError> {
    let retired = store.retired_sticks()?;
    for stick in store.owned_sticks()? {
        for old in &retired {
            remark_drive(&stick, old)?;
        }
    }
    clean_up_retired(store, secrets, data_dir)
}

fn clean_up_retired(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<(), LifeError> {
    for old in store.retired_sticks()? {
        delete_secrets(secrets, &old)?;
        remove_cache(data_dir, &old)?;
        store.forget_retired(&old)?;
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
    // Copied files: only an app stick's, only behind its marker; a disk
    // image's go with the image.
    if stick.kind == StickKind::App
        && stick.image_path.is_none()
        && drive_present(&stick.root_path, &stick.stick_id)
    {
        empty_drive(&stick.root_path)?;
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

/// Deletes what is in a drive's folder, then its marker, then the folder if
/// it is empty: never following a link, never crossing into another disk
/// mounted inside (left in place). The marker goes last, so a removal
/// stopped halfway still finds it.
fn empty_drive(root: &Path) -> Result<(), LifeError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| LifeError::Io { path, source }
    };
    let disk = crate::linked::disk_of(&std::fs::metadata(root).map_err(io(root))?);
    for entry in std::fs::read_dir(root).map_err(io(root))? {
        let entry = entry.map_err(io(root))?;
        if entry.file_name() == DRIVE_MARKER {
            continue;
        }
        delete_tree(&entry.path(), disk)?;
    }
    let marker = root.join(DRIVE_MARKER);
    std::fs::remove_file(&marker).map_err(io(&marker))?;
    // Not empty: something on another disk was left; that is the user's.
    let _ = std::fs::remove_dir(root);
    Ok(())
}

fn delete_tree(path: &Path, disk: Option<u64>) -> Result<(), LifeError> {
    let io = |source| LifeError::Io {
        path: path.to_path_buf(),
        source,
    };
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_dir() {
        return std::fs::remove_file(path).map_err(io);
    }
    if disk.is_some() && crate::linked::disk_of(&metadata) != disk {
        // Another disk mounted inside the stick: not the stick's.
        return Ok(());
    }
    for entry in std::fs::read_dir(path).map_err(io)? {
        delete_tree(&entry.map_err(io)?.path(), disk)?;
    }
    // Not empty if something inside was on another disk.
    let _ = std::fs::remove_dir(path);
    Ok(())
}

#[cfg(test)]
mod tests;
