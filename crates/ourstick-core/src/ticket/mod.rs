//! A ticket is everything a reader needs to open a stick.
//!
//! It has a public part (who and where) and a secret part (the key), and is
//! shared as `https://ourstick.com/s/<public>#<secret>`. The secret lives only in
//! the URL fragment. See `docs/protocol.md` for the byte layout.
//!
//! A ticket grants read access to whoever holds it. It must never be logged:
//! `Debug` hides the key, and there is no `Display`, only [`Ticket::to_link`].

mod encoding;

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::cipher::StickKey;
use crate::record::ManifestHint;
use crate::stick_id::StickId;

pub use encoding::{LINK_PREFIX, TICKET_FORMAT_VERSION};

/// Maximum number of address hints in a ticket.
pub const MAX_ADDRESS_HINTS: usize = 8;

/// Maximum length of a relay URL in a ticket, in bytes.
pub const MAX_RELAY_URL_LEN: usize = 255;

/// Maximum length of the public part of a ticket, in bytes.
pub const MAX_PUBLIC_PART_LEN: usize = 1024;

/// The iroh node id of the owner: the node's Ed25519 public key.
///
/// Kept as bytes here so the core does not depend on iroh's types; the
/// transport converts it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub [u8; 32]);

impl fmt::Debug for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = data_encoding::HEXLOWER.encode(&self.0);
        write!(formatter, "NodeId({hex})")
    }
}

/// Where the owner's node may be reached. Only a hint: the node is identified
/// by its [`NodeId`], and addresses go stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressHint {
    Ipv4 { ip: [u8; 4], port: u16 },
    Ipv6 { ip: [u8; 16], port: u16 },
    Relay { url: String },
}

impl From<SocketAddr> for AddressHint {
    fn from(address: SocketAddr) -> Self {
        match address.ip() {
            IpAddr::V4(ip) => Self::Ipv4 {
                ip: ip.octets(),
                port: address.port(),
            },
            IpAddr::V6(ip) => Self::Ipv6 {
                ip: ip.octets(),
                port: address.port(),
            },
        }
    }
}

/// Everything a reader needs to open a stick.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket {
    // Public part.
    pub stick_id: StickId,
    pub node_id: NodeId,
    /// The signed record's `seq` and manifest hash when the ticket was made.
    /// A hint for the first download, and the oldest record a reader accepts;
    /// the latest signed record is the authority.
    pub manifest_hint: ManifestHint,
    pub address_hints: Vec<AddressHint>,
    // Secret part.
    pub stick_key: StickKey,
}

impl fmt::Debug for Ticket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Ticket")
            .field("stick_id", &self.stick_id)
            .field("node_id", &self.node_id)
            .field("manifest_seq", &self.manifest_hint.seq)
            .field(
                "manifest_hash",
                &data_encoding::HEXLOWER.encode(&self.manifest_hint.hash),
            )
            .field("address_hints", &self.address_hints)
            .field("stick_key", &"<redacted>")
            .finish()
    }
}

/// Why a ticket could not be read or written. The messages are meant for users.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    #[error("this is not an OurStick ticket (expected {LINK_PREFIX}…#…)")]
    NotATicket,
    #[error("the ticket is too long to be an OurStick ticket")]
    TooLong,
    #[error("the ticket has no key after '#'; was the whole link copied?")]
    MissingSecret,
    #[error("the ticket is damaged or incomplete; ask for it again")]
    PublicPartDamaged,
    #[error("the key part of the ticket is damaged, incomplete, or from another ticket")]
    SecretPartDamaged,
    #[error("this ticket was made by a newer version of OurStick (format {0}); update OurStick")]
    UnsupportedVersion(u8),
    #[error("the ticket contains an invalid stick id")]
    InvalidStickId,
    #[error("the ticket has {0} address hints, at most {MAX_ADDRESS_HINTS} are allowed")]
    TooManyAddressHints(usize),
    #[error("the ticket contains an invalid relay URL")]
    InvalidRelayUrl,
    #[error("the ticket's public part is {0} bytes, at most {MAX_PUBLIC_PART_LEN} are allowed")]
    PublicPartTooLong(usize),
}

impl Ticket {
    /// Checks the rules a writer must follow and a reader enforces.
    fn validate(&self) -> Result<(), TicketError> {
        if self.address_hints.len() > MAX_ADDRESS_HINTS {
            return Err(TicketError::TooManyAddressHints(self.address_hints.len()));
        }
        for hint in &self.address_hints {
            if let AddressHint::Relay { url } = hint {
                let printable = url.bytes().all(|byte| (0x21..=0x7E).contains(&byte));
                if url.is_empty() || url.len() > MAX_RELAY_URL_LEN || !printable {
                    return Err(TicketError::InvalidRelayUrl);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
