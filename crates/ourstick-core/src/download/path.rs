//! Which path the data arrives on.
//!
//! With QUIC multipath each side picks the path it *sends* on. A reader sends
//! almost nothing (requests, acknowledgements), so the path it has selected can
//! stay on the relay while the owner sends all the data over a direct path. The
//! first transfer between two real machines showed exactly that: every line
//! said "relay" and 4.6 GB of 4.6 GB arrived directly. So the reader judges the
//! path by where received bytes come from.

use crate::transport::{PathKind, PathUsage};

/// How much data must arrive between two checks. Checking by amount, not time,
/// keeps this free of clocks.
pub(super) const CHECK_EVERY: u64 = 4 * 1024 * 1024;

pub(super) struct ReceivingPath {
    kind: PathKind,
    last_usage: PathUsage,
    unchecked: u64,
}

impl ReceivingPath {
    pub(super) fn new(initial: PathKind, usage: PathUsage) -> Self {
        Self {
            kind: initial,
            last_usage: usage,
            unchecked: 0,
        }
    }

    /// Call with the bytes received since the last call. Every
    /// [`CHECK_EVERY`] bytes it reads `usage` and returns the new path if the
    /// data now mostly arrives over a different one.
    pub(super) fn on_received(
        &mut self,
        bytes: u64,
        usage: impl FnOnce() -> PathUsage,
    ) -> Option<PathKind> {
        self.unchecked += bytes;
        if self.unchecked < CHECK_EVERY {
            return None;
        }
        self.unchecked = 0;
        let now = usage();
        let direct = now.direct.saturating_sub(self.last_usage.direct);
        let relay = now.relay.saturating_sub(self.last_usage.relay);
        tracing::debug!(
            total_direct = now.direct,
            total_relay = now.relay,
            new_direct = direct,
            new_relay = relay,
            "receiving path check"
        );
        self.last_usage = now;
        let kind = match direct.cmp(&relay) {
            std::cmp::Ordering::Greater => PathKind::Direct,
            std::cmp::Ordering::Less => PathKind::Relay,
            std::cmp::Ordering::Equal => return None,
        };
        if kind == self.kind {
            None
        } else {
            self.kind = kind;
            Some(kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(direct: u64, relay: u64) -> PathUsage {
        PathUsage {
            direct,
            relay,
            direct_ipv6: 0,
        }
    }

    #[test]
    fn reports_direct_when_data_arrives_directly_despite_a_relay_start() {
        let mut path = ReceivingPath::new(PathKind::Relay, usage(0, 10_000));
        // Not enough data yet: no check, no reading of stats.
        assert_eq!(
            path.on_received(CHECK_EVERY - 1, || panic!("checked too early")),
            None
        );
        assert_eq!(
            path.on_received(1, || usage(5_000_000, 12_000)),
            Some(PathKind::Direct)
        );
        // Still direct: nothing to report.
        assert_eq!(
            path.on_received(CHECK_EVERY, || usage(10_000_000, 13_000)),
            None
        );
    }

    #[test]
    fn reports_a_fall_back_to_the_relay() {
        let mut path = ReceivingPath::new(PathKind::Direct, usage(0, 0));
        assert_eq!(
            path.on_received(CHECK_EVERY, || usage(1_000, 4_000_000)),
            Some(PathKind::Relay)
        );
    }

    #[test]
    fn no_new_data_keeps_the_current_path() {
        let mut path = ReceivingPath::new(PathKind::Relay, usage(7, 7));
        assert_eq!(path.on_received(CHECK_EVERY, || usage(7, 7)), None);
    }
}
