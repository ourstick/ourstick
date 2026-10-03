//! What the window hears from a stick's share: its activity and its
//! problems, each naming its stick.

use ourstick_core::transport::ReaderActivity;
use ourstick_core::StickId;
use serde::Serialize;

/// A problem with sharing, as the UI hears of it (event `share-problem`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareProblem {
    /// Could not publish a change; tried again later.
    Failed { message: String },
    /// The stick's folder is not there; the last version is still served.
    Ejected,
    /// Some of the stick's folders cannot be watched (e.g. too many
    /// folders): changes are seen only now and then.
    SeenSlowly,
    /// Publishing works again.
    Cleared,
}

/// OurStick began or stopped giving way to another program using the disk
/// (event `giving-way`, ADR 0013), told at each change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GivingWayView {
    pub giving_way: bool,
}

/// An event about one stick (`activity`, `share-problem`, `giving-way`): the window
/// shows only its own stick's.
#[derive(Debug, Clone, Serialize)]
pub struct StickEvent<T: Serialize> {
    pub stick_id: String,
    #[serde(flatten)]
    pub event: T,
}

impl<T: Serialize> StickEvent<T> {
    pub fn new(stick_id: &StickId, event: T) -> Self {
        Self {
            stick_id: crate::state::stick_hex(stick_id),
            event,
        }
    }
}

/// Who downloads (event `activity`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActivityView {
    pub readers: usize,
    pub bytes_per_second: u64,
    /// Readers wait while their files are got ready (ADR 0007): nothing
    /// leaves, and that is no fault.
    pub preparing: bool,
    /// What to do when readers reach this computer only through the relay
    /// (see [`relay_only_message`]); `None` otherwise.
    pub relay_only: Option<&'static str>,
}

impl From<ReaderActivity> for ActivityView {
    fn from(activity: ReaderActivity) -> Self {
        Self {
            readers: activity.readers,
            bytes_per_second: activity.bytes_per_second,
            preparing: activity.preparing,
            relay_only: activity.only_relayed.then(relay_only_message),
        }
    }
}

/// Readers reach this computer only through the relay: most likely its
/// firewall drops what they send (on Windows, after "Cancel" on the
/// Windows Firewall question). Says so, and how to fix it here.
pub fn relay_only_message() -> &'static str {
    if cfg!(windows) {
        "People can download only through the relay, which is slower. Windows \
         Firewall probably blocks OurStick: open Windows Security, Firewall & \
         network protection, Allow an app through firewall, and tick OurStick \
         for the network you are on."
    } else if cfg!(target_os = "macos") {
        "People can download only through the relay, which is slower. The \
         firewall probably blocks OurStick: open System Settings, Network, \
         Firewall, Options, and allow incoming connections for OurStick."
    } else {
        "People can download only through the relay, which is slower. A \
         firewall probably drops what they send: allow incoming UDP for \
         OurStick (e.g. in ufw or firewalld)."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_names_its_stick_as_the_window_does() {
        let id = ourstick_core::StickSigningKey::from_bytes(&[4; 32]).stick_id();
        let event = StickEvent::new(&id, ShareProblem::Ejected);
        assert_eq!(event.stick_id, crate::state::stick_hex(&id));
        assert_eq!(event.stick_id.len(), 64);
    }
}
