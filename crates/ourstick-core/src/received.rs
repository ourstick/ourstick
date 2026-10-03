//! What this device receives (step 7b): which links it takes, where a
//! received stick's blobs wait, and forgetting a received stick. Forgetting
//! keeps the files already received (they are the user's now) and the
//! stick's record floor (ADR 0004: the same link added again must not take
//! an older version).

use std::path::{Path, PathBuf};

use crate::locks::{download_lock, Lock, LockError};
use crate::secrets::{SecretError, SecretName, SecretStore};
use crate::stick_id::StickId;
use crate::store::{JoinedChange, StickKind, Store, StoreError};

/// Why a link is not taken, or a received stick not forgotten. The messages
/// are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum ReceivedError {
    #[error("this is one of your own sticks (\u{201c}{0}\u{201d})")]
    OwnStick(String),
    #[error("this was one of your own sticks, before you changed its sharing link")]
    RetiredStick,
    #[error("you already receive this stick (\u{201c}{0}\u{201d})")]
    AlreadyReceived(String),
    #[error("OurStick (the app) receives this stick, into {}; use the app, or forget the stick there first", .0.display())]
    ReceivedByApp(PathBuf),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// This computer's reader key for a received stick, if it has one: none
/// means the owner never heard from it, so a pause or a forget need not
/// be told (and no key is made just to say so).
pub fn reader_key_if_any(
    secrets: &dyn SecretStore,
    stick_id: &StickId,
) -> Result<Option<crate::report::ReaderKey>, SecretError> {
    Ok(secrets
        .get(&SecretName::ReaderKey(*stick_id))?
        .and_then(|seed| <[u8; 32]>::try_from(seed.as_slice()).ok())
        .map(|seed| crate::report::ReaderKey::from_bytes(&seed)))
}

/// This computer's reader key for a received stick (ADR 0015): the one
/// kept in the secret store, or a new one kept there now (a stick received
/// before reports, or its first round). Never in a plain file.
pub fn reader_key(
    secrets: &dyn SecretStore,
    stick_id: &StickId,
) -> Result<crate::report::ReaderKey, SecretError> {
    let name = SecretName::ReaderKey(*stick_id);
    if let Some(seed) = secrets.get(&name)? {
        if let Ok(seed) = <[u8; 32]>::try_from(seed.as_slice()) {
            return Ok(crate::report::ReaderKey::from_bytes(&seed));
        }
        // Damaged: a new key only changes which line the owner shows.
        tracing::warn!("a received stick's reader key was damaged; a new one is made");
    }
    let key = crate::report::ReaderKey::generate();
    secrets.put(&name, &key.to_bytes())?;
    Ok(key)
}

/// Whether the program `kind` may receive the stick `stick_id`: never one of
/// this device's own sticks (or one it gave a new sharing link); for the
/// receiver window, not one it already receives (it takes over one `get`
/// received); for `get`, not one the window receives.
pub fn check_link(
    store: &dyn Store,
    stick_id: &StickId,
    kind: StickKind,
) -> Result<(), ReceivedError> {
    let own = store.owned_stick(stick_id)?.or(store
        .removing_sticks()?
        .into_iter()
        .find(|stick| stick.stick_id == *stick_id));
    if let Some(own) = own {
        return Err(ReceivedError::OwnStick(own.name));
    }
    if store
        .retired_sticks()?
        .iter()
        .any(|retired| retired.old == *stick_id)
    {
        return Err(ReceivedError::RetiredStick);
    }
    match store.joined_stick(stick_id)? {
        Some(joined) if joined.kind == StickKind::App && kind == StickKind::App => Err(
            ReceivedError::AlreadyReceived(display_name(joined.name.as_deref(), stick_id)),
        ),
        Some(joined) if joined.kind == StickKind::App => {
            Err(ReceivedError::ReceivedByApp(joined.output_dir))
        }
        _ => Ok(()),
    }
}

/// A received stick's name for a message: as its manifest gave it, else
/// "Stick" and the first digits of its id.
pub fn display_name(name: Option<&str>, stick_id: &StickId) -> String {
    match name {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => {
            let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
            format!("Stick {}", &hex[..8])
        }
    }
}

/// Where a received stick's blobs wait between batches and runs: one folder
/// per stick in the data folder, for every program.
pub fn received_store_dir(data_dir: &Path, stick_id: &StickId) -> PathBuf {
    data_dir
        .join("joined")
        .join(data_encoding::HEXLOWER.encode(&stick_id.to_bytes()))
}

/// "Forget this stick": no longer received, its sharing link (ticket)
/// forgotten, its waiting blobs and records deleted; the files already
/// received stay. `_held` is the stick's download lock (no other program
/// receives it meanwhile); the caller has stopped receiving it. Marked
/// first, so a crash leaves it hidden and [`finish_forgetting`] completes it
/// at the next start.
pub fn forget_received(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick_id: &StickId,
    _held: &Lock,
) -> Result<(), ReceivedError> {
    match store.change_joined_stick(stick_id, &JoinedChange::Forgetting) {
        Ok(()) => {}
        // Marked already (a forget stopped halfway): finish it.
        Err(StoreError::StickNotFound)
            if store
                .forgetting_joined_sticks()?
                .iter()
                .any(|stick| stick.stick_id == *stick_id) => {}
        Err(error) => return Err(error.into()),
    }
    finish(store, secrets, data_dir, stick_id)
}

/// At start, before anything is received: every forget stopped halfway is
/// finished; one that fails again stays, and says why. A stick whose
/// download lock is held is being forgotten right now, and is left to it.
pub fn finish_forgetting(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<Vec<(String, ReceivedError)>, ReceivedError> {
    let mut failed = Vec::new();
    for stick in store.forgetting_joined_sticks()? {
        let name = display_name(stick.name.as_deref(), &stick.stick_id);
        let _lock = match download_lock(data_dir, &stick.stick_id) {
            Ok(lock) => lock,
            Err(LockError::DownloadInUse) => continue,
            Err(error) => {
                failed.push((name, error.into()));
                continue;
            }
        };
        if let Err(error) = finish(store, secrets, data_dir, &stick.stick_id) {
            failed.push((name, error));
        }
    }
    Ok(failed)
}

/// The rest of a forget, each step done again harmlessly.
fn finish(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    stick_id: &StickId,
) -> Result<(), ReceivedError> {
    let folder = store
        .forgetting_joined_sticks()?
        .into_iter()
        .find(|stick| stick.stick_id == *stick_id)
        .map(|stick| stick.output_dir);
    secrets.delete(&SecretName::Ticket(*stick_id))?;
    secrets.delete(&SecretName::ReaderKey(*stick_id))?;
    let blobs = received_store_dir(data_dir, stick_id);
    match std::fs::remove_dir_all(&blobs) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ReceivedError::Io {
                path: blobs,
                source,
            })
        }
    }
    store.forget_joined_stick(stick_id)?;
    // Last, and only tried: the files are the user's now, without OurStick's
    // mark among them.
    if let Some(folder) = folder {
        crate::download::unmark_received_folder(&folder, stick_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
