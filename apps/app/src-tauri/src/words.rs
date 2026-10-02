//! Errors as the window says them: one place decides. What came from the
//! keyring or OurStick's own records is said in words for everyone (the
//! details go to the log); everything else keeps its whole explanation,
//! which names what to do (an image in the way, a drive letter taken).

use ourstick_core::device::DeviceError;
use ourstick_core::{SecretError, StoreError};

pub(crate) const SECRETS: &str = "OurStick could not reach the passwords this computer keeps \
    for it. Unlock them, or allow OurStick when it asks, then try again.";
pub(crate) const RECORDS: &str = "OurStick's own records on this computer could not be read or \
    written. Try again in a moment.";

/// Words for the kinds of error people cannot act on as written; `None`
/// for the rest. Logs what it replaced.
pub(crate) fn known(error: &anyhow::Error) -> Option<String> {
    let words = if error.downcast_ref::<SecretError>().is_some() {
        SECRETS
    } else if let Some(device) = error.downcast_ref::<DeviceError>() {
        match device {
            DeviceError::Secret(_) => SECRETS,
            DeviceError::Store(_) => RECORDS,
            _ => return None,
        }
    } else if error.downcast_ref::<StoreError>().is_some() {
        RECORDS
    } else {
        return None;
    };
    tracing::warn!(error = %format!("{error:#}"), "said in the window in words");
    Some(words.to_owned())
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
    fn the_rest_keeps_its_whole_explanation() {
        let mount =
            anyhow::anyhow!("hdiutil: attach failed - image in use").context("cannot mount");
        assert_eq!(
            told(&mount),
            "cannot mount: hdiutil: attach failed - image in use"
        );
    }
}
