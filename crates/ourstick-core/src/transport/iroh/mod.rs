//! [`Publisher`](super::Publisher) and [`Fetcher`](super::Fetcher) on iroh 1.x
//! and iroh-blobs. This is the only place in the workspace that uses them.

mod activity;
mod batches;
mod fetcher;
mod handler;
mod local_network;
mod made_store;
mod publisher;
mod reader_paths;
mod record_protocol;
mod report_protocol;
mod serving;

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, SecretKey, TransportAddr};

use super::TransportError;
use crate::local_network::{LocalNetwork, SystemLocalNetwork};
use crate::ticket::{AddressHint, NodeId, Ticket, MAX_ADDRESS_HINTS};

pub use fetcher::{IrohConnection, IrohFetcher};
pub use publisher::IrohPublisher;
pub use record_protocol::RECORD_ALPN;
pub use report_protocol::{IrohReportSender, REPORT_ALPN};

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
    relays: Relays,
    /// Asked whether macOS declines the local network (ADR 0020).
    local_network: Arc<dyn LocalNetwork>,
}

/// Which relays a [`Network::Public`] node uses (ADR 0006: configurable, not
/// tied to n0). Further relays are added next to n0's, never instead of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Relays {
    /// The public relays run by n0 (iroh's default).
    #[default]
    N0Public,
    /// n0's public relays and these ones, e.g. a future
    /// `https://relay.ourstick.com`.
    N0PublicAnd(Vec<String>),
}

impl IrohConfig {
    /// `secret_key` is this device's node key, kept in the secret store.
    pub fn new(secret_key: [u8; 32], network: Network) -> Self {
        Self {
            secret_key,
            network,
            relays: Relays::default(),
            local_network: Arc::new(SystemLocalNetwork),
        }
    }

    pub(super) fn local_network(&self) -> Arc<dyn LocalNetwork> {
        self.local_network.clone()
    }

    /// What tells whether the local network is declined (a fake in tests).
    pub fn with_local_network(mut self, local_network: Arc<dyn LocalNetwork>) -> Self {
        self.local_network = local_network;
        self
    }

    /// Which relays to use with [`Network::Public`].
    pub fn with_relays(mut self, relays: Relays) -> Self {
        self.relays = relays;
        self
    }

    /// Which relay this configuration uses, for the user.
    pub fn relay_description(&self) -> String {
        match (&self.network, &self.relays) {
            (Network::LocalOnly, _) => "no relay (local only)".to_owned(),
            (Network::Public, Relays::N0Public) => "the public iroh relays run by n0".to_owned(),
            (Network::Public, Relays::N0PublicAnd(urls)) => {
                format!("the public iroh relays run by n0 and {}", urls.join(", "))
            }
        }
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
            .field("relays", &self.relays)
            .finish_non_exhaustive()
    }
}

async fn bind(config: &IrohConfig, alpns: Vec<Vec<u8>>) -> Result<Endpoint, TransportError> {
    let mut builder = match config.network {
        // Relay connections go through an HTTP(S) proxy if the environment
        // names one (ADR 0006).
        Network::Public => Endpoint::builder(presets::N0).proxy_from_env(),
        Network::LocalOnly => Endpoint::builder(presets::Minimal),
    };
    if let (Network::Public, Relays::N0PublicAnd(extra)) = (&config.network, &config.relays) {
        let mut urls: Vec<RelayUrl> = iroh::defaults::prod::default_relay_map().urls();
        for url in extra {
            urls.push(
                url.parse::<RelayUrl>()
                    .map_err(|_| TransportError::Bind(format!("invalid relay URL: {url}")))?,
            );
        }
        builder = builder.relay_mode(iroh::RelayMode::custom(urls));
    }
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

/// This endpoint's latest network report, in our own terms.
/// Waits up to `timeout` for the first complete network report, then returns
/// what there is (`None` if the measurement has not finished).
async fn wait_network_report(
    endpoint: &Endpoint,
    timeout: std::time::Duration,
) -> Option<super::NetworkReport> {
    use iroh::Watcher as _;
    let mut watcher = endpoint.net_report();
    // A timeout is not an error: the caller says the report is not ready.
    let _ = tokio::time::timeout(timeout, watcher.initialized()).await;
    network_report(endpoint)
}

fn network_report(endpoint: &Endpoint) -> Option<super::NetworkReport> {
    use iroh::Watcher as _;
    let report = endpoint.net_report().get()?;
    Some(super::NetworkReport {
        udp_v4: report.udp_v4,
        udp_v6: report.udp_v6,
        ipv4_mapping_varies: report.mapping_varies_by_dest_ipv4,
        public_v4: report.global_v4.map(SocketAddr::V4),
        public_v6: report.global_v6.map(SocketAddr::V6),
    })
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
    fn relay_description_names_the_relay() {
        let key = [1; 32];
        assert_eq!(
            IrohConfig::new(key, Network::Public).relay_description(),
            "the public iroh relays run by n0"
        );
        assert_eq!(
            IrohConfig::new(key, Network::Public)
                .with_relays(Relays::N0PublicAnd(
                    vec!["https://relay.example.org".into()]
                ))
                .relay_description(),
            "the public iroh relays run by n0 and https://relay.example.org"
        );
        assert_eq!(
            IrohConfig::new(key, Network::LocalOnly).relay_description(),
            "no relay (local only)"
        );
    }

    #[tokio::test]
    async fn invalid_custom_relay_is_a_clear_error() {
        let config = IrohConfig::new([1; 32], Network::Public)
            .with_relays(Relays::N0PublicAnd(vec!["not a url".into()]));
        assert!(matches!(
            bind(&config, Vec::new()).await,
            Err(TransportError::Bind(message)) if message.contains("not a url")
        ));
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
