//! Which of a link's address hints a reader dials.

use std::net::SocketAddr;

use iroh::RelayUrl;

/// Whether a link's address hint may be dialled. A link is written by
/// whoever sends it: an address no owner has (none, every host, a group,
/// port 0) or a cloud's own (169.254.x.x, where its metadata service
/// answers) only makes this computer send packets where the link says.
pub(super) fn may_dial(socket: &SocketAddr) -> bool {
    let ip = socket.ip();
    socket.port() != 0
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !matches!(ip, std::net::IpAddr::V4(v4) if v4.is_broadcast() || v4.is_link_local())
}

/// A link's relay hint, if it names a relay as relays are named: `https`,
/// a host name (not an address, not `localhost`), no user. Anything else
/// would only make this computer ask a host of the link's choosing.
pub(super) fn relay_hint(url: &str) -> Option<RelayUrl> {
    let relay = url.parse::<RelayUrl>().ok()?;
    let host = relay.host_str()?.trim_matches(['[', ']']);
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let named = relay.scheme() == "https"
        && relay.username().is_empty()
        && relay.password().is_none()
        && host.parse::<std::net::IpAddr>().is_err()
        && host.contains('.')
        && host != "localhost"
        && !host.ends_with(".localhost");
    named.then_some(relay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_no_owner_has_are_never_dialled() {
        for hint in [
            "0.0.0.0:4433",
            "224.0.0.1:4433",
            "255.255.255.255:4433",
            "169.254.169.254:80",
            "10.0.0.1:0",
            "[::]:4433",
            "[ff02::1]:4433",
        ] {
            assert!(!may_dial(&hint.parse().unwrap()), "{hint}");
        }
        // Loopback and private ones stay: the same computer, the same network.
        for hint in ["127.0.0.1:4433", "192.168.1.20:4433", "[2001:db8::1]:4433"] {
            assert!(may_dial(&hint.parse().unwrap()), "{hint}");
        }
        for url in [
            "http://relay.example.org/",
            "https://10.0.0.5:8443/",
            "https://[::1]/",
            "https://localhost/",
            "https://a.localhost/",
            "https://user@relay.example.org/",
            "https://intranet/",
        ] {
            assert!(relay_hint(url).is_none(), "{url}");
        }
        assert!(relay_hint("https://euc1-1.relay.n0.iroh.link./").is_some());
    }
}
