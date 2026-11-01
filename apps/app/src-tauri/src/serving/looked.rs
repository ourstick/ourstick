//! When each shared stick was last looked at (ADR 0029 decision 4). A
//! phone has no watcher: it looks again now and then, when the app comes
//! back to the front and on "Look again", and its view says when it last
//! looked. Kept in memory: a new run looks again at once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ourstick_core::StickId;

/// The time of the last look at each stick, in seconds since 1970.
#[derive(Default)]
pub struct LastLooked(Mutex<HashMap<StickId, u64>>);

impl LastLooked {
    /// `stick_id` was looked at just now.
    pub fn looked(&self, stick_id: &StickId) {
        self.set(stick_id, SystemTime::now());
    }

    fn set(&self, stick_id: &StickId, at: SystemTime) {
        let seconds = at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(*stick_id, seconds);
    }

    /// When `stick_id` was last looked at, if it was in this run.
    pub fn of(&self, stick_id: &StickId) -> Option<u64> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(stick_id)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn stick(byte: u8) -> StickId {
        ourstick_core::StickSigningKey::from_bytes(&[byte; 32]).stick_id()
    }

    #[test]
    fn each_stick_keeps_its_last_look() {
        let looked = LastLooked::default();
        assert_eq!(looked.of(&stick(1)), None);
        looked.set(&stick(1), UNIX_EPOCH + Duration::from_secs(100));
        looked.set(&stick(2), UNIX_EPOCH + Duration::from_secs(50));
        looked.set(&stick(1), UNIX_EPOCH + Duration::from_secs(160));
        assert_eq!(looked.of(&stick(1)), Some(160));
        assert_eq!(looked.of(&stick(2)), Some(50));
    }

    #[test]
    fn a_look_just_now_is_after_the_start_of_today() {
        let looked = LastLooked::default();
        looked.looked(&stick(3));
        assert!(looked
            .of(&stick(3))
            .is_some_and(|seconds| seconds > 1_700_000_000));
    }
}
