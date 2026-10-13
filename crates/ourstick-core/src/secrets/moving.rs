//! Moving a device's secrets from one store to another (ADR 0021, rule 3):
//! on macOS from the login keychain to the data protection keychain. Every
//! secret is copied and read back before anything is recorded; nothing is
//! ever made anew (CLAUDE.md rule 9). Values stay in memory and are never
//! logged; a secret is named by its key (stick ids), never its value.

use super::{SecretError, SecretName, SecretStore};
use crate::store::{Store, StoreError};

/// The secrets a device's database says exist.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ToMove {
    /// Must be there: a stick's keys, a received stick's link, the device's
    /// node key if it has one.
    pub required: Vec<SecretName>,
    /// May be there: a received stick's reader key (made at its first
    /// report), and the keys of what is being removed or was replaced.
    pub optional: Vec<SecretName>,
}

impl ToMove {
    fn all(&self) -> impl Iterator<Item = (&SecretName, bool)> {
        let required = self.required.iter().map(|name| (name, true));
        required.chain(self.optional.iter().map(|name| (name, false)))
    }
}

/// Why secrets could not be moved. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("the key {0} is not where it should be; nothing was moved")]
    Missing(String),
    #[error("the key {0} is already in the new store with another value; nothing was moved")]
    Differs(String),
    #[error("the key {0} did not read back as written; nothing was moved")]
    NotKept(String),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Every secret of the device `store` describes.
pub fn secrets_to_move(store: &dyn Store) -> Result<ToMove, StoreError> {
    let mut to_move = ToMove::default();
    if store
        .device()?
        .is_some_and(|device| device.node_id.is_some())
    {
        to_move.required.push(SecretName::NodeKey);
    }
    for stick in store.owned_sticks()? {
        to_move
            .required
            .push(SecretName::StickSigningKey(stick.stick_id));
        to_move.required.push(SecretName::StickKey(stick.stick_id));
        if stick.node_id.is_some() {
            to_move
                .required
                .push(SecretName::StickNodeKey(stick.stick_id));
        }
    }
    // Being removed: what is left of their keys goes along, to be deleted
    // from where the device then keeps them.
    for stick in store.removing_sticks()? {
        to_move
            .optional
            .push(SecretName::StickSigningKey(stick.stick_id));
        to_move.optional.push(SecretName::StickKey(stick.stick_id));
        to_move
            .optional
            .push(SecretName::StickNodeKey(stick.stick_id));
    }
    for retired in store.retired_sticks()? {
        to_move
            .optional
            .push(SecretName::StickSigningKey(retired.old));
        to_move.optional.push(SecretName::StickKey(retired.old));
        to_move.optional.push(SecretName::StickNodeKey(retired.old));
    }
    let forgetting: Vec<_> = store
        .forgetting_joined_sticks()?
        .into_iter()
        .map(|stick| stick.stick_id)
        .collect();
    for joined in store.joined_sticks()? {
        // Only the app keeps a received stick's link; `get` is given it.
        if forgetting.contains(&joined.stick_id) || joined.kind != crate::store::StickKind::App {
            to_move.optional.push(SecretName::Ticket(joined.stick_id));
        } else {
            to_move.required.push(SecretName::Ticket(joined.stick_id));
        }
        to_move
            .optional
            .push(SecretName::ReaderKey(joined.stick_id));
    }
    // A link the app waits for (ADR 0025) keeps its ticket as a received
    // stick of the app does.
    for waiting in store.waiting_links()? {
        to_move.required.push(SecretName::Ticket(waiting.stick_id));
    }
    for stick in forgetting {
        if !to_move.optional.contains(&SecretName::Ticket(stick)) {
            to_move.optional.push(SecretName::Ticket(stick));
            to_move.optional.push(SecretName::ReaderKey(stick));
        }
    }
    Ok(to_move)
}

/// Copies every secret of `to_move` from `from` to `to`, each read back and
/// compared; one already in `to` with the same value is left as it is. Any
/// missing required secret, or one in `to` with another value, stops it
/// before anything is written: then neither store is changed. Returns how
/// many were written.
pub fn copy_secrets(
    from: &dyn SecretStore,
    to: &dyn SecretStore,
    to_move: &ToMove,
) -> Result<usize, MoveError> {
    // First every check, so a stop writes nothing.
    let mut writes = Vec::new();
    for (name, required) in to_move.all() {
        let Some(value) = from.get(name)? else {
            if required {
                return Err(MoveError::Missing(name.as_key()));
            }
            continue;
        };
        match to.get(name)? {
            Some(there) if there == value => {}
            Some(_) => return Err(MoveError::Differs(name.as_key())),
            None => writes.push((name, value)),
        }
    }
    for (name, value) in &writes {
        to.put(name, value)?;
        if to.get(name)?.as_ref() != Some(value) {
            return Err(MoveError::NotKept(name.as_key()));
        }
    }
    Ok(writes.len())
}

/// Whether every required secret of `to_move` reads from `store`: before the
/// old copies go.
pub fn all_there(store: &dyn SecretStore, to_move: &ToMove) -> Result<(), MoveError> {
    for name in &to_move.required {
        if store.get(name)?.is_none() {
            return Err(MoveError::Missing(name.as_key()));
        }
    }
    Ok(())
}

/// Deletes the old copies, once [`all_there`] said the new store has them.
pub fn delete_old(from: &dyn SecretStore, to_move: &ToMove) -> Result<(), SecretError> {
    for (name, _) in to_move.all() {
        from.delete(name)?;
    }
    Ok(())
}

/// Moves this device's keys from `from` (the login keychain) to `to` (the
/// data protection keychain): every one copied and read back, then `to`
/// recorded as their store, the old copies still there (deleted by
/// [`finish_move`] at a later start). A failure records nothing: the keys
/// stay where they were, and are read from there. Take
/// [`crate::locks::setup_lock`] first. Returns how many were written.
pub fn move_keys(
    store: &dyn Store,
    from: &dyn SecretStore,
    to: &dyn SecretStore,
) -> Result<usize, MoveError> {
    let to_move = secrets_to_move(store)?;
    let written = copy_secrets(from, to, &to_move)?;
    store.record_keys_moved(true)?;
    Ok(written)
}

/// At a start after [`move_keys`]: once every key reads from `to`, the old
/// copies in `from` go. True if they went (or none were left).
pub fn finish_move(
    store: &dyn Store,
    from: &dyn SecretStore,
    to: &dyn SecretStore,
) -> Result<bool, MoveError> {
    if !store.old_keys_left()? {
        return Ok(true);
    }
    let to_move = secrets_to_move(store)?;
    all_there(to, &to_move)?;
    delete_old(from, &to_move)?;
    store.record_keys_moved(false)?;
    Ok(true)
}

#[cfg(test)]
#[path = "moving_tests.rs"]
mod tests;
