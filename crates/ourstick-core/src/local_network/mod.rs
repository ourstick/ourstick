//! Whether macOS keeps this app from reaching its local network (ADR 0020):
//! declined, its sends to an address on an attached network fail at once,
//! and a peer on the same network can only be reached through a relay.

mod same_network;

pub use same_network::same_network;

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

/// What a probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The system refuses what this app sends to its local network.
    Declined,
    /// Not refused: allowed, or not answered yet (the send itself may be
    /// what makes macOS ask).
    NotDeclined,
    /// It cannot be told here: no private IPv4 address, or another error.
    Unknown,
}

/// Asks whether the local network is declined; a trait so tests need no
/// network. Blocking: call it from `spawn_blocking`.
pub trait LocalNetwork: Send + Sync {
    fn access(&self) -> Access;
}

/// The real system: probed on macOS, never declined elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLocalNetwork;

impl LocalNetwork for SystemLocalNetwork {
    fn access(&self) -> Access {
        if cfg!(target_os = "macos") {
            match own_ipv4().and_then(probe_target) {
                Some(target) => classify(&probe(target)),
                None => Access::Unknown,
            }
        } else {
            Access::NotDeclined
        }
    }
}

/// The port probes are sent to: discard, which nothing answers.
const DISCARD: u16 = 9;
/// Between a probe's two sends.
const SECOND_SEND_AFTER: Duration = Duration::from_millis(100);

/// This computer's IPv4 address towards the internet, as the routing table
/// chooses it. Connecting a UDP socket sends nothing.
pub fn own_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket
        .connect((Ipv4Addr::new(192, 0, 2, 1), DISCARD))
        .ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Where to probe from `own`: another address of its /24, `.253` (or
/// `.252` if that is its own), chosen to be neither this computer nor,
/// usually, its router (which macOS lets every app reach). `None` unless
/// `own` is private (RFC 1918).
pub fn probe_target(own: Ipv4Addr) -> Option<Ipv4Addr> {
    if !own.is_private() {
        return None;
    }
    let [a, b, c, d] = own.octets();
    Some(Ipv4Addr::new(a, b, c, if d == 253 { 252 } else { 253 }))
}

/// Sends twice to `target` from a connected socket: each send's result.
pub fn probe(target: Ipv4Addr) -> Vec<io::Result<()>> {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
        socket
            .connect(SocketAddrV4::new(target, DISCARD))
            .map(|_| socket)
    }) {
        Ok(socket) => socket,
        Err(error) => return vec![Err(error)],
    };
    let first = socket.send(b"ourstick").map(|_| ());
    std::thread::sleep(SECOND_SEND_AFTER);
    let second = socket.send(b"ourstick").map(|_| ());
    vec![first, second]
}

/// macOS's `EHOSTDOWN`: no computer answered for the address (ARP).
const HOST_DOWN: i32 = 64;

/// What a probe's sends say (seen on macOS 26, ADR 0020): a connected
/// socket's sends are refused with "broken pipe" while the permission is
/// declined (or not answered yet). Allowed, they go, and to an address no
/// computer has they fail later as host down or unreachable; "connection
/// refused" answers an earlier send. Declined only if every send was
/// refused so; not declined only if every one went or failed so.
pub fn classify(sends: &[io::Result<()>]) -> Access {
    let refused = |result: &io::Result<()>| matches!(result, Err(error) if error.kind() == io::ErrorKind::BrokenPipe);
    let went = |result: &io::Result<()>| match result {
        Ok(()) => true,
        Err(error) => {
            matches!(
                error.kind(),
                io::ErrorKind::HostUnreachable | io::ErrorKind::ConnectionRefused
            ) || error.raw_os_error() == Some(HOST_DOWN)
        }
    };
    if sends.is_empty() {
        Access::Unknown
    } else if sends.iter().all(refused) {
        Access::Declined
    } else if sends.iter().all(went) {
        Access::NotDeclined
    } else {
        Access::Unknown
    }
}

#[cfg(test)]
mod tests;
