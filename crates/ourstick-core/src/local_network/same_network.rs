//! Whether a peer is on this computer's network (ADR 0020, rule 4): one of
//! its addresses private IPv4 in the same /24 as one of ours, or IPv6 (not
//! link-local) in the same /64; our own addresses are not a peer's (the
//! same computer). Two homes on the same router defaults (192.168.1.x
//! both) are told apart by their public IPv4 addresses, when both are
//! known: one network is behind one. A heuristic: a wider network is
//! missed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether any of `theirs` is on the network of one of `ours`.
pub fn same_network(ours: &[IpAddr], theirs: &[IpAddr]) -> bool {
    let ours_public: Vec<&IpAddr> = ours.iter().filter(|ip| public_v4(ip)).collect();
    let theirs_public: Vec<&IpAddr> = theirs.iter().filter(|ip| public_v4(ip)).collect();
    if !ours_public.is_empty()
        && !theirs_public.is_empty()
        && !theirs_public
            .iter()
            .any(|theirs| ours_public.contains(theirs))
    {
        return false;
    }
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

/// A public IPv4 address: not private, loopback, link-local, unspecified
/// or the shared 100.64.0.0/10 (carrier NAT, Tailscale).
fn public_v4(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(_) => false,
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
        assert!(!same_network(
            &ips(&["203.0.113.10"]),
            &ips(&["203.0.113.11"])
        ));
        // Tailscale's shared range is not a local network.
        assert!(!same_network(
            &ips(&["100.64.0.27"]),
            &ips(&["100.64.0.30"])
        ));
        assert!(!same_network(&ips(&["192.168.1.20"]), &[]));
        assert!(!same_network(&[], &ips(&["192.168.1.20"])));
    }

    #[test]
    fn two_homes_on_the_same_router_defaults_are_told_apart_by_their_public_address() {
        // Both 192.168.1.x, behind different public addresses.
        assert!(!same_network(
            &ips(&["192.168.1.20", "203.0.113.10"]),
            &ips(&["192.168.1.23", "198.51.100.7"])
        ));
        // Behind the same one: one network.
        assert!(same_network(
            &ips(&["192.168.1.20", "203.0.113.10"]),
            &ips(&["192.168.1.23", "203.0.113.10"])
        ));
        // One side's public address unknown: by the /24 alone.
        assert!(same_network(
            &ips(&["192.168.1.20"]),
            &ips(&["192.168.1.23", "198.51.100.7"])
        ));
        // A carrier NAT's shared range is not a public address.
        assert!(same_network(
            &ips(&["192.168.1.20", "100.70.1.1"]),
            &ips(&["192.168.1.23", "100.80.2.2"])
        ));
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
