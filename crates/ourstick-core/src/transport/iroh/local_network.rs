//! ADR 0020 over iroh: whether a peer is on this computer's network, from
//! what iroh knows of both, and whether a declined Local Network permission
//! is what keeps a connection on the relay.

use std::net::IpAddr;
use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, TransportAddr};

use super::fetcher::{current_path, IrohFetcher};
use crate::local_network::{same_network, Access, DeclinedHere, SharedProbe, LOOK_EVERY};
use crate::ticket::{AddressHint, Ticket};
use crate::transport::PathKind;

/// Whether `peer` is on this computer's network: its addresses as iroh
/// learned them (over the relay too; perhaps stale) against this
/// endpoint's own.
pub(super) async fn on_this_network(endpoint: &Endpoint, peer: EndpointId) -> bool {
    let ours: Vec<IpAddr> = endpoint.addr().ip_addrs().map(|addr| addr.ip()).collect();
    let theirs: Vec<IpAddr> = match endpoint.remote_info(peer).await {
        Some(info) => info
            .addrs()
            .filter_map(|addr| match addr.addr() {
                TransportAddr::Ip(addr) => Some(addr.ip()),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };
    same_network(&ours, &theirs)
}

impl IrohFetcher {
    /// Whether the owner of `ticket` is on this computer's network, by the
    /// ticket's address hints (ADR 0020).
    pub fn on_this_network(&self, ticket: &Ticket) -> bool {
        let ours: Vec<IpAddr> = self
            .endpoint
            .addr()
            .ip_addrs()
            .map(|addr| addr.ip())
            .collect();
        let theirs: Vec<IpAddr> = ticket
            .address_hints
            .iter()
            .filter_map(|hint| match hint {
                AddressHint::Ipv4 { ip, .. } => Some((*ip).into()),
                AddressHint::Ipv6 { ip, .. } => Some((*ip).into()),
                AddressHint::Relay { .. } => None,
            })
            .collect();
        same_network(&ours, &theirs)
    }
}

/// Tells `tell` whenever it changes whether a declined permission keeps
/// `connection`'s peer on the relay (rule 5), looking every
/// [`LOOK_EVERY`], until aborted.
pub(super) async fn watch_declined(
    connection: Connection,
    endpoint: Endpoint,
    probe: Arc<SharedProbe>,
    mut tell: impl FnMut(bool),
) {
    let mut declined = DeclinedHere::default();
    let mut said = false;
    let mut ticker = tokio::time::interval(LOOK_EVERY);
    loop {
        // Each look at its tick's time, so ten seconds of facts are told at
        // the second look, not the third; until the connection closes.
        let at = tokio::select! {
            at = ticker.tick() => at.into_std(),
            _ = connection.closed() => return,
        };
        let relay = current_path(&connection) == PathKind::Relay;
        let same = relay && on_this_network(&endpoint, connection.remote_id()).await;
        // The probe only when it could matter.
        let access = if same {
            probe.access().await
        } else {
            Access::Unknown
        };
        let now = declined.check(same, relay, access, at);
        if now != said {
            said = now;
            tell(now);
        }
    }
}
