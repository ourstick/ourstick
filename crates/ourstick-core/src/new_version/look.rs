//! When to look, and what a look keeps (ADR 0033 rules 1, 5 and 10): once a
//! day at a random minute, again an hour after a failure, and at start if
//! the last good look is a day old.

use rand::Rng;

use super::keys::NoticeKeys;
use super::notice::{read_notice, Notice, System};
use super::source::{NoticeSource, MOST_BYTES};
use super::state::{LookFailure, NoticeStore, VersionLook};
use crate::clock::Clock;
use crate::store::StoreError;

pub const HOUR_MS: u64 = 60 * 60 * 1000;
pub const DAY_MS: u64 = 24 * HOUR_MS;

/// Due: looking is on, and the next look's time has come, or lies further
/// ahead than any schedule puts it (the clock was moved back).
pub fn is_due(look: &VersionLook, now_ms: u64) -> bool {
    look.looking
        && (look.next_look_ms <= now_ms
            || look.next_look_ms > now_ms.saturating_add(DAY_MS + HOUR_MS))
}

/// Looks if due; true if it looked. The file's contents are never logged.
pub async fn look_if_due<S: NoticeSource>(
    store: &dyn NoticeStore,
    source: &S,
    clock: &dyn Clock,
    keys: &NoticeKeys,
    system: System,
) -> Result<bool, StoreError> {
    if !is_due(&store.version_look()?, clock.now_ms()) {
        return Ok(false);
    }
    let outcome = match source.read().await {
        Ok(bytes) if bytes.len() > MOST_BYTES => Err(LookFailure::NotTheFile),
        Ok(bytes) => read_notice(&bytes, keys, system).map_err(|error| {
            tracing::info!(%error, "version.json not taken");
            LookFailure::NotTheFile
        }),
        Err(error) => {
            tracing::info!(%error, "could not read version.json");
            Err(error.failure())
        }
    };
    // Read again: looking may have been turned off while the read went on.
    let mut look = store.version_look()?;
    if !look.looking {
        return Ok(false);
    }
    let now = clock.now_ms();
    match outcome {
        Ok(notice) => {
            take(&mut look, notice);
            look.last_good_ms = Some(now);
            look.failing_since_ms = None;
            look.failure = None;
            look.next_look_ms = now + DAY_MS + rand::thread_rng().gen_range(0..HOUR_MS);
        }
        Err(failure) => {
            look.failing_since_ms.get_or_insert(now);
            look.failure = Some(failure);
            look.next_look_ms = now + HOUR_MS;
        }
    }
    store.set_version_look(&look)?;
    Ok(true)
}

/// Keeps what a good notice says, unless its key has signed a higher serial
/// (an old notice served again cannot hide a newer one).
fn take(look: &mut VersionLook, notice: Notice) {
    let key_id = notice.key_id().to_string();
    if look
        .serials
        .get(&key_id)
        .is_some_and(|&taken| notice.serial() < taken)
    {
        tracing::info!("an older notice than the one taken; ignored");
        return;
    }
    look.serials.insert(key_id, notice.serial());
    match notice {
        Notice::Versions {
            newest, security, ..
        } => {
            look.newest = Some(newest.to_string());
            look.security = Some(security.to_string());
        }
        Notice::ReleaseKeyChanged {
            new_release_key, ..
        } => look.new_release_key = Some(new_release_key),
    }
}

/// "Look for new versions" on or off. Off forgets a newer version, but not
/// a security line nor a key change (rule 10); on looks at once.
pub fn set_looking(store: &dyn NoticeStore, on: bool) -> Result<(), StoreError> {
    let mut look = store.version_look()?;
    look.looking = on;
    look.next_look_ms = 0;
    if !on {
        look.newest = None;
        look.failing_since_ms = None;
        look.failure = None;
    }
    store.set_version_look(&look)
}

/// The window told the user that the app now looks (rule 11).
pub fn told_looking(store: &dyn NoticeStore) -> Result<(), StoreError> {
    let mut look = store.version_look()?;
    look.looking_told = true;
    store.set_version_look(&look)
}

/// The window told of this security notice once (rule 8).
pub fn told_security(store: &dyn NoticeStore) -> Result<(), StoreError> {
    let mut look = store.version_look()?;
    look.security_told = look.security.clone();
    store.set_version_look(&look)
}

#[cfg(test)]
#[path = "look_tests.rs"]
mod tests;
