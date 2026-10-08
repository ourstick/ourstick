//! Why data goes through a relay instead of directly (CLAUDE.md: every network
//! failure says why).
//!
//! Built from what the transport measured about this device's network and what
//! the owner offered in the ticket. Pure, so it is tested without a network.

use std::net::SocketAddr;

use crate::ticket::AddressHint;

/// What this device found out about its own network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkReport {
    /// UDP over IPv4 reaches the internet.
    pub udp_v4: bool,
    /// UDP over IPv6 reaches the internet.
    pub udp_v6: bool,
    /// The public IPv4 port differs per destination (symmetric NAT or carrier
    /// grade NAT), which defeats IPv4 hole punching.
    pub ipv4_mapping_varies: Option<bool>,
    /// The public IPv4 address and port others see.
    pub public_v4: Option<SocketAddr>,
    /// The public IPv6 address and port others see.
    pub public_v6: Option<SocketAddr>,
}

/// One line describing this device's network, e.g. for `share` at start.
pub fn describe_network(report: &NetworkReport) -> String {
    let ipv4 = match (report.udp_v4, report.ipv4_mapping_varies) {
        (false, _) => "IPv4: no UDP".to_owned(),
        (true, Some(true)) => {
            "IPv4: behind a NAT that changes ports per destination (hard to reach directly)"
                .to_owned()
        }
        (true, _) => match report.public_v4 {
            Some(address) => format!("IPv4: public {address}"),
            None => "IPv4: works".to_owned(),
        },
    };
    let ipv6 = match (report.udp_v6, report.public_v6) {
        (false, _) => "IPv6: none".to_owned(),
        (true, Some(address)) => format!("IPv6: {}", address.ip()),
        (true, None) => "IPv6: works".to_owned(),
    };
    format!("{ipv4}; {ipv6}")
}

/// Reasons, most likely first, why the data comes through a relay. `local` is
/// this device's report (`None` while it is still being measured);
/// `on_this_network`, whether the owner is on this computer's network (ADR
/// 0020): then a Mac's declined Local Network permission, on either side,
/// is the likeliest.
pub fn relay_reasons(
    local: Option<&NetworkReport>,
    owner_hints: &[AddressHint],
    on_this_network: bool,
) -> Vec<String> {
    let owner_has_ipv6 = owner_hints
        .iter()
        .any(|hint| matches!(hint, AddressHint::Ipv6 { .. }));
    let mut reasons = Vec::new();
    if on_this_network {
        reasons.push(
            "the owner is on this network: if it is a Mac, it may need to allow OurStick \
             in System Settings → Privacy & Security → Local Network"
                .to_owned(),
        );
    }
    let Some(local) = local else {
        reasons.push("this device has not finished checking its network yet".to_owned());
        return reasons;
    };
    if !local.udp_v4 && !local.udp_v6 {
        reasons.push(
            "UDP does not get through on this network; only the relay (over HTTPS) works"
                .to_owned(),
        );
        return reasons;
    }
    if local.udp_v6 && owner_has_ipv6 {
        reasons.push(
            "both sides have IPv6, but no direct IPv6 path opened: a firewall on one side \
             (the owner's router or this network's provider) drops incoming UDP"
                .to_owned(),
        );
    } else if local.udp_v6 {
        reasons
            .push("the owner offered no IPv6 address, so the direct IPv6 route is out".to_owned());
    } else if owner_has_ipv6 {
        reasons.push(
            "this network has no IPv6, so the owner's IPv6 address cannot be used".to_owned(),
        );
    }
    match (local.udp_v4, local.ipv4_mapping_varies) {
        (false, _) => reasons.push("UDP over IPv4 does not get through on this network".to_owned()),
        (true, Some(true)) => reasons.push(
            "this network's IPv4 NAT changes the public port for every destination \
             (typical for mobile carriers), which defeats IPv4 hole punching"
                .to_owned(),
        ),
        _ => {}
    }
    reasons.push(
        "the owner can make direct connections reliable by enabling UPnP (or NAT-PMP/PCP) \
         on its router, so its side can be reached from outside"
            .to_owned(),
    );
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv6_hint() -> AddressHint {
        AddressHint::Ipv6 {
            ip: [0x2a, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            port: 4433,
        }
    }

    fn ipv4_hint() -> AddressHint {
        AddressHint::Ipv4 {
            ip: [2, 84, 198, 32],
            port: 4433,
        }
    }

    /// This device on a phone hotspot: CGNAT on IPv4, working IPv6.
    fn hotspot() -> NetworkReport {
        NetworkReport {
            udp_v4: true,
            udp_v6: true,
            ipv4_mapping_varies: Some(true),
            public_v4: Some("198.51.100.2:40000".parse().unwrap()),
            public_v6: Some("[2001:db8::89]:5000".parse().unwrap()),
        }
    }

    #[test]
    fn hotspot_with_an_ipv6_owner_blames_firewalls_and_cgnat() {
        let reasons = relay_reasons(Some(&hotspot()), &[ipv4_hint(), ipv6_hint()], false);
        assert!(reasons[0].contains("both sides have IPv6"), "{reasons:?}");
        assert!(
            reasons[1].contains("changes the public port"),
            "{reasons:?}"
        );
        assert!(reasons.last().unwrap().contains("UPnP"), "{reasons:?}");
    }

    #[test]
    fn owner_without_ipv6_is_named() {
        let reasons = relay_reasons(Some(&hotspot()), &[ipv4_hint()], false);
        assert!(reasons[0].contains("owner offered no IPv6"), "{reasons:?}");
    }

    #[test]
    fn missing_ipv6_here_is_named() {
        let local = NetworkReport {
            udp_v6: false,
            public_v6: None,
            ..hotspot()
        };
        let reasons = relay_reasons(Some(&local), &[ipv6_hint()], false);
        assert!(
            reasons[0].contains("this network has no IPv6"),
            "{reasons:?}"
        );
    }

    #[test]
    fn no_udp_at_all_is_the_only_reason() {
        let reasons = relay_reasons(Some(&NetworkReport::default()), &[ipv6_hint()], false);
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("UDP does not get through"));
    }

    #[test]
    fn unknown_report_says_so() {
        let reasons = relay_reasons(None, &[], false);
        assert_eq!(
            reasons,
            vec!["this device has not finished checking its network yet"]
        );
    }

    #[test]
    fn an_owner_on_this_network_names_a_macs_local_network_first() {
        let reasons = relay_reasons(Some(&hotspot()), &[ipv4_hint()], true);
        assert!(reasons[0].contains("Local Network"));
        assert!(relay_reasons(None, &[], true)[0].contains("Local Network"));
    }

    #[test]
    fn network_is_described_in_one_line() {
        assert_eq!(
            describe_network(&hotspot()),
            "IPv4: behind a NAT that changes ports per destination (hard to reach directly); \
             IPv6: 2001:db8::89"
        );
        let home = NetworkReport {
            udp_v4: true,
            udp_v6: false,
            ipv4_mapping_varies: Some(false),
            public_v4: Some("203.0.113.32:53419".parse().unwrap()),
            public_v6: None,
        };
        assert_eq!(
            describe_network(&home),
            "IPv4: public 203.0.113.32:53419; IPv6: none"
        );
    }
}
