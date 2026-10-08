//! Which path the data arrives on.
//!
//! With QUIC multipath each side picks the path it *sends* on. A reader sends
//! almost nothing (requests, acknowledgements), so the path it has selected can
//! stay on the relay while the owner sends all the data over a direct path. The
//! first transfer between two real machines showed exactly that: every line
//! said "relay" and 4.6 GB of 4.6 GB arrived directly. So the reader judges the
//! path by where received bytes come from.

use std::time::Instant;

use super::relay_watch::RelayWatch;
use crate::transport::{PathKind, PathUsage};

/// How much data must arrive between two checks. Checking by amount, not time,
/// keeps this free of clocks.
pub(super) const CHECK_EVERY: u64 = 4 * 1024 * 1024;

/// The result of one check: where the latest data came from, and whether that
/// differs from what was reported before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Check {
    pub(super) kind: PathKind,
    pub(super) changed: bool,
    /// Relay bytes received on this connection so far.
    pub(super) relay_total: u64,
    /// On the relay long enough: connect again to look for a direct path.
    pub(super) look_again: bool,
}

pub(super) struct ReceivingPath<'a> {
    kind: PathKind,
    last_usage: PathUsage,
    unchecked: u64,
    /// Kept over the connections of one download.
    watch: &'a mut RelayWatch,
}

impl<'a> ReceivingPath<'a> {
    pub(super) fn new(initial: PathKind, usage: PathUsage, watch: &'a mut RelayWatch) -> Self {
        Self {
            kind: initial,
            last_usage: usage,
            unchecked: 0,
            watch,
        }
    }

    /// Whether the connection says a declined local network keeps it on the
    /// relay (ADR 0020): `Some` when it changed since last told.
    pub(super) fn declined(&mut self, now: bool) -> Option<bool> {
        self.watch.declined(now)
    }

    /// A new connection is being made to look for a direct path.
    pub(super) fn looked(&mut self) {
        self.watch.looked();
    }

    /// Call with the bytes received since the last call. Every
    /// [`CHECK_EVERY`] bytes it reads `usage` and returns which kind of path
    /// the data since the previous check mostly came over.
    pub(super) fn on_received(
        &mut self,
        bytes: u64,
        usage: impl FnOnce() -> PathUsage,
    ) -> Option<Check> {
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
        let changed = kind != self.kind;
        self.kind = kind;
        let look_again = self.watch.on_check(kind == PathKind::Relay, Instant::now());
        Some(Check {
            kind,
            changed,
            relay_total: now.relay,
            look_again,
        })
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
        let mut watch = RelayWatch::default();
        let mut path = ReceivingPath::new(PathKind::Relay, usage(0, 10_000), &mut watch);
        // Not enough data yet: no check, no reading of stats.
        assert_eq!(
            path.on_received(CHECK_EVERY - 1, || panic!("checked too early")),
            None
        );
        assert_eq!(
            path.on_received(1, || usage(5_000_000, 12_000)),
            Some(Check {
                kind: PathKind::Direct,
                changed: true,
                relay_total: 12_000,
                look_again: false,
            })
        );
        // Still direct: checked, nothing changed.
        assert_eq!(
            path.on_received(CHECK_EVERY, || usage(10_000_000, 13_000)),
            Some(Check {
                kind: PathKind::Direct,
                changed: false,
                relay_total: 13_000,
                look_again: false,
            })
        );
    }

    #[test]
    fn reports_a_fall_back_to_the_relay() {
        let mut watch = RelayWatch::default();
        let mut path = ReceivingPath::new(PathKind::Direct, usage(0, 0), &mut watch);
        assert_eq!(
            path.on_received(CHECK_EVERY, || usage(1_000, 4_000_000)),
            Some(Check {
                kind: PathKind::Relay,
                changed: true,
                relay_total: 4_000_000,
                look_again: false,
            })
        );
    }

    #[test]
    fn no_new_data_keeps_the_current_path() {
        let mut watch = RelayWatch::default();
        let mut path = ReceivingPath::new(PathKind::Relay, usage(7, 7), &mut watch);
        assert_eq!(path.on_received(CHECK_EVERY, || usage(7, 7)), None);
    }
}
