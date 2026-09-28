//! [`Publisher`](super::Publisher) and [`Fetcher`](super::Fetcher) on iroh 1.x
//! and iroh-blobs. This is the only place in the workspace that uses them.

mod fetcher;
mod publisher;
mod record_protocol;

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, SecretKey, TransportAddr};

use super::TransportError;
use crate::ticket::{AddressHint, NodeId, Ticket, MAX_ADDRESS_HINTS};

pub use fetcher::{IrohConnection, IrohFetcher};
pub use publisher::IrohPublisher;
pub use record_protocol::RECORD_ALPN;

/// Which networks the node uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// Public iroh relays and address lookup (DNS/pkarr via n0). For real use.
    Public,
    /// Direct connections only: no relays, no address lookup. For tests on one
    /// machine or one network.
    LocalOnly,
}

/// Settings for an iroh node. `Debug` never prints the secret key.
#[derive(Clone)]
pub struct IrohConfig {
    secret_key: [u8; 32],
    network: Network,
    gc_interval: Duration,
}

impl IrohConfig {
    /// `secret_key` is this device's node key, kept in the secret store.
    pub fn new(secret_key: [u8; 32], network: Network) -> Self {
        Self {
            secret_key,
            network,
            gc_interval: Duration::from_secs(60),
        }
    }

    /// How often the owner's cache removes blobs that are no longer served.
    pub fn with_gc_interval(mut self, interval: Duration) -> Self {
        self.gc_interval = interval;
        self
    }

    /// The node id that belongs to a secret key.
    pub fn node_id_of(secret_key: &[u8; 32]) -> NodeId {
        NodeId(*SecretKey::from_bytes(secret_key).public().as_bytes())
    }
}

impl fmt::Debug for IrohConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IrohConfig")
            .field("node_id", &Self::node_id_of(&self.secret_key))
            .field("network", &self.network)
            .field("gc_interval", &self.gc_interval)
            .finish_non_exhaustive()
    }
}

async fn bind(config: &IrohConfig, alpns: Vec<Vec<u8>>) -> Result<Endpoint, TransportError> {
    let builder = match config.network {
        Network::Public => Endpoint::builder(presets::N0),
        Network::LocalOnly => Endpoint::builder(presets::Minimal),
    };
    builder
        .secret_key(SecretKey::from_bytes(&config.secret_key))
        .alpns(alpns)
        .bind()
        .await
        .map_err(|error| TransportError::Bind(error.to_string()))
}

fn node_id(endpoint: &Endpoint) -> NodeId {
    NodeId(*endpoint.id().as_bytes())
}

/// The endpoint's current addresses as ticket hints: relay first, then direct
/// addresses (IPv6 included), at most [`MAX_ADDRESS_HINTS`].
fn address_hints(endpoint: &Endpoint) -> Vec<AddressHint> {
    let addr = endpoint.addr();
    let relays = addr.relay_urls().map(|url| AddressHint::Relay {
        url: url.to_string(),
    });
    let direct = addr.ip_addrs().map(|ip| AddressHint::from(*ip));
    relays.chain(direct).take(MAX_ADDRESS_HINTS).collect()
}

/// The owner's address from a ticket. Hints that do not parse are skipped: the
/// node id alone is enough when address lookup is on.
fn owner_addr(ticket: &Ticket) -> Result<EndpointAddr, TransportError> {
    let key = PublicKey::from_bytes(&ticket.node_id.0)
        .map_err(|_| TransportError::Unreachable("the ticket has an invalid node id".into()))?;
    let mut addr = EndpointAddr::new(key);
    for hint in &ticket.address_hints {
        match hint {
            AddressHint::Ipv4 { ip, port } => {
                addr = addr.with_ip_addr(SocketAddr::from((*ip, *port)));
            }
            AddressHint::Ipv6 { ip, port } => {
                addr = addr.with_ip_addr(SocketAddr::from((*ip, *port)));
            }
            AddressHint::Relay { url } => match url.parse::<RelayUrl>() {
                Ok(url) => addr = addr.with_relay_url(url),
                Err(_) => tracing::debug!("skipping relay hint that does not parse"),
            },
        }
    }
    Ok(addr)
}

/// iroh-blobs wants absolute paths for imports and exports.
fn absolute(path: &Path) -> Result<PathBuf, TransportError> {
    Ok(std::path::absolute(path)?)
}

/// True if any address in the set is a relay (used to describe paths).
fn is_relay(addr: &TransportAddr) -> bool {
    matches!(addr, TransportAddr::Relay(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::StickKey;
    use crate::record::ManifestHint;
    use crate::stick_id::StickSigningKey;

    fn ticket(hints: Vec<AddressHint>) -> Ticket {
        Ticket {
            stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
            node_id: IrohConfig::node_id_of(&[2; 32]),
            manifest_hint: ManifestHint {
                seq: 1,
                hash: [0; 32],
            },
            address_hints: hints,
            stick_key: StickKey::from_bytes([3; 32]),
        }
    }

    #[test]
    fn owner_addr_takes_every_valid_hint() {
        let addr = owner_addr(&ticket(vec![
            "10.0.0.1:4433".parse::<SocketAddr>().unwrap().into(),
            "[2001:db8::1]:4433".parse::<SocketAddr>().unwrap().into(),
            AddressHint::Relay {
                url: "https://relay.example.org./".into(),
            },
            AddressHint::Relay {
                url: "not a url".into(),
            },
        ]))
        .unwrap();
        assert_eq!(addr.id.as_bytes(), &IrohConfig::node_id_of(&[2; 32]).0);
        assert_eq!(addr.ip_addrs().count(), 2);
        assert_eq!(addr.relay_urls().count(), 1);
    }

    #[test]
    fn owner_addr_rejects_invalid_node_id() {
        let mut bad = ticket(Vec::new());
        bad.node_id = NodeId([0; 32]);
        bad.node_id.0[0] = 2; // y = 2 is not on the curve
        assert!(matches!(
            owner_addr(&bad),
            Err(TransportError::Unreachable(_))
        ));
    }

    #[test]
    fn config_debug_hides_secret_key() {
        let config = IrohConfig::new([0xAB; 32], Network::LocalOnly);
        let printed = format!("{config:?}");
        assert!(!printed.contains(&"ab".repeat(8)), "{printed}");
        assert!(!printed.contains("171"), "{printed}");
    }

    #[test]
    fn node_id_is_derived_from_secret_key() {
        assert_eq!(
            IrohConfig::node_id_of(&[2; 32]),
            IrohConfig::node_id_of(&[2; 32])
        );
        assert_ne!(
            IrohConfig::node_id_of(&[2; 32]),
            IrohConfig::node_id_of(&[3; 32])
        );
    }
}
