//! Whether a peer is on this computer's network (ADR 0020, rule 4): one of
//! its addresses private IPv4 in the same /24 as one of ours, or IPv6 (not
//! link-local) in the same /64; our own addresses are not a peer's (the
//! same computer). A heuristic: a wider network is missed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether any of `theirs` is on the network of one of `ours`.
pub fn same_network(ours: &[IpAddr], theirs: &[IpAddr]) -> bool {
    theirs
        .iter()
        .filter(|theirs| !ours.contains(theirs))
        .any(|theirs| ours.iter().any(|ours| neighbours(ours, theirs)))
}

fn neighbours(ours: &IpAddr, theirs: &IpAddr) -> bool {
    match (ours, theirs) {
        (IpAddr::V4(ours), IpAddr::V4(theirs)) => {
            ours.is_private() && theirs.is_private() && same_24(ours, theirs)
        }
        (IpAddr::V6(ours), IpAddr::V6(theirs)) => {
            local_or_global(ours) && local_or_global(theirs) && same_64(ours, theirs)
        }
        _ => false,
    }
}

fn same_24(a: &Ipv4Addr, b: &Ipv4Addr) -> bool {
    a.octets()[..3] == b.octets()[..3]
}

fn same_64(a: &Ipv6Addr, b: &Ipv6Addr) -> bool {
    a.segments()[..4] == b.segments()[..4]
}

/// A unique local (fc00::/7) or global address: not loopback, unspecified,
/// link-local (fe80::/10) or multicast.
fn local_or_global(ip: &Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && (first & 0xffc0) != 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(list: &[&str]) -> Vec<IpAddr> {
        list.iter().map(|ip| ip.parse().unwrap()).collect()
    }

    #[test]
    fn a_private_neighbour_in_the_same_24_is_on_this_network() {
        assert!(same_network(
            &ips(&["192.168.1.20"]),
            &ips(&["192.168.1.23"])
        ));
        assert!(same_network(
            &ips(&["10.0.7.4", "2001:db8:9::5"]),
            &ips(&["8.8.8.8", "10.0.7.200"])
        ));
    }

    #[test]
    fn other_networks_are_not() {
        // Another /24 of the same /16: missed, as the heuristic says.
        assert!(!same_network(
            &ips(&["192.168.1.20"]),
            &ips(&["192.168.2.10"])
        ));
        // Public addresses, even in one /24.
        assert!(!same_network(&ips(&["203.0.113.10"]), &ips(&["203.0.113.11"])));
        // Tailscale's shared range is not a local network.
        assert!(!same_network(
            &ips(&["100.64.0.27"]),
            &ips(&["100.64.0.30"])
        ));
        assert!(!same_network(&ips(&["192.168.1.20"]), &[]));
        assert!(!same_network(&[], &ips(&["192.168.1.20"])));
    }

    #[test]
    fn our_own_address_is_the_same_computer() {
        assert!(!same_network(
            &ips(&["192.168.1.20"]),
            &ips(&["192.168.1.20"])
        ));
    }

    #[test]
    fn ipv6_in_the_same_64_is_on_this_network_but_link_local_is_not() {
        assert!(same_network(
            &ips(&["2001:db8:1:2::10"]),
            &ips(&["2001:db8:1:2:aa::1"])
        ));
        assert!(same_network(
            &ips(&["fd00:1:2:3::1"]),
            &ips(&["fd00:1:2:3::2"])
        ));
        assert!(!same_network(
            &ips(&["2001:db8:1:2::10"]),
            &ips(&["2001:db8:1:3::10"])
        ));
        assert!(!same_network(&ips(&["fe80::1"]), &ips(&["fe80::2"])));
    }

    #[test]
    fn ipv4_and_ipv6_are_never_neighbours() {
        assert!(!same_network(&ips(&["192.168.1.20"]), &ips(&["fd00::1"])));
    }
}
