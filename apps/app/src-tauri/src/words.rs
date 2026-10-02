//! Errors as the window says them: one place decides. What came from the
//! keyring or OurStick's own records is said in words for everyone (the
//! details go to the log); everything else keeps its whole explanation,
//! which names what to do (an image in the way, a drive letter taken).

use std::error::Error;

use ourstick_core::device::DeviceError;
use ourstick_core::received::ReceivedError;
use ourstick_core::share::ShareError;
use ourstick_core::stick_life::LifeError;
use ourstick_core::{SecretError, StoreError};

pub(crate) const SECRETS: &str = "OurStick could not reach the passwords this computer keeps \
    for it. Unlock them, or allow OurStick when it asks, then try again.";
pub(crate) const RECORDS: &str = "OurStick's own records on this computer could not be read or \
    written. Try again in a moment.";

/// Words for the kinds of error people cannot act on as written (the
/// keyring, OurStick's own records), wherever in the chain and however
/// wrapped; `None` for the rest. Logs what it replaced.
pub(crate) fn known(error: &anyhow::Error) -> Option<String> {
    let words = error.chain().find_map(kind)?;
    tracing::warn!(error = %format!("{error:#}"), "said in the window in words");
    Some(words.to_owned())
}

/// An error the core reports by reference (a watcher's), as the window
/// shows it.
pub(crate) fn told_one(error: &(dyn Error + 'static)) -> String {
    match kind(error) {
        Some(words) => {
            tracing::warn!(%error, "said in the window in words");
            words.to_owned()
        }
        None => error.to_string(),
    }
}

/// The words for one error of the chain. The core's wrappers pass the
/// keyring's and the store's errors on transparently, so they are looked
/// into here (their `source` skips the wrapped error itself).
fn kind(error: &(dyn Error + 'static)) -> Option<&'static str> {
    if error.is::<SecretError>() {
        return Some(SECRETS);
    }
    if error.is::<StoreError>() {
        return Some(RECORDS);
    }
    if let Some(device) = error.downcast_ref::<DeviceError>() {
        return match device {
            DeviceError::Secret(_) => Some(SECRETS),
            DeviceError::Store(_) => Some(RECORDS),
            _ => None,
        };
    }
    if let Some(life) = error.downcast_ref::<LifeError>() {
        return match life {
            LifeError::Secret(_) => Some(SECRETS),
            LifeError::Store(_) => Some(RECORDS),
            LifeError::Share(share) => kind(share),
            _ => None,
        };
    }
    if let Some(ShareError::Store(_)) = error.downcast_ref::<ShareError>() {
        return Some(RECORDS);
    }
    if let Some(received) = error.downcast_ref::<ReceivedError>() {
        return match received {
            ReceivedError::Secret(_) => Some(SECRETS),
            ReceivedError::Store(_) => Some(RECORDS),
            _ => None,
        };
    }
    None
}

/// An error of the user's own sticks, as the window shows it.
pub(crate) fn told(error: &anyhow::Error) -> String {
    known(error).unwrap_or_else(|| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked() -> SecretError {
        SecretError::AccessDenied {
            store: "the macOS keychain".to_owned(),
            reason: "User interaction is not allowed.".to_owned(),
        }
    }

    #[test]
    fn the_keyring_and_the_records_are_said_in_words() {
        // As opening the keyring gives it, with a context on top.
        let opening = anyhow::Error::from(DeviceError::from(locked())).context("cannot start");
        assert_eq!(told(&opening), SECRETS);
        assert_eq!(told(&anyhow::Error::from(locked())), SECRETS);
        let records = anyhow::Error::from(StoreError::StickNotFound);
        assert_eq!(told(&records), RECORDS);
    }

    #[test]
    fn wrapped_by_the_core_they_are_found_too() {
        let removal = anyhow::Error::from(LifeError::Secret(locked()));
        assert_eq!(told(&removal), SECRETS);
        let sharing = anyhow::Error::from(ShareError::Store(StoreError::StickNotFound));
        assert_eq!(told(&sharing), RECORDS);
        let nested = anyhow::Error::from(LifeError::Share(ShareError::Store(
            StoreError::StickNotFound,
        )));
        assert_eq!(told(&nested), RECORDS);
        let forget = anyhow::Error::from(ReceivedError::Secret(locked()));
        assert_eq!(told(&forget), SECRETS);
        // A key that is missing says to restore it, not to unlock.
        let missing = anyhow::Error::from(DeviceError::KeyMissing {
            name: "node-key".to_owned(),
            store: "the keyring".to_owned(),
        });
        assert!(told(&missing).contains("Restore it"));
    }

    #[test]
    fn the_rest_keeps_its_whole_explanation() {
        let mount =
            anyhow::anyhow!("hdiutil: attach failed - image in use").context("cannot mount");
        assert_eq!(
            told(&mount),
            "cannot mount: hdiutil: attach failed - image in use"
        );
    }
}
