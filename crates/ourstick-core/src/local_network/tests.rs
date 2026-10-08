use super::*;

fn error(kind: io::ErrorKind) -> io::Result<()> {
    Err(io::Error::from(kind))
}

#[test]
fn every_send_refused_as_a_broken_pipe_is_declined() {
    let refused = || error(io::ErrorKind::BrokenPipe);
    assert_eq!(classify(&[refused(), refused()]), Access::Declined);
}

#[test]
fn sends_that_went_or_found_no_computer_are_not_declined() {
    assert_eq!(classify(&[Ok(()), Ok(())]), Access::NotDeclined);
    // Allowed, to an address nobody has: the router's ARP gave up.
    let down = Err(io::Error::from_raw_os_error(HOST_DOWN));
    let unreachable = error(io::ErrorKind::HostUnreachable);
    assert_eq!(classify(&[unreachable, down]), Access::NotDeclined);
    // The first answered by "port unreachable": it went.
    let refused = error(io::ErrorKind::ConnectionRefused);
    assert_eq!(classify(&[Ok(()), refused]), Access::NotDeclined);
}

#[test]
fn anything_else_cannot_be_told() {
    assert_eq!(classify(&[]), Access::Unknown);
    let mixed = [Ok(()), error(io::ErrorKind::BrokenPipe)];
    assert_eq!(classify(&mixed), Access::Unknown);
    let other = [error(io::ErrorKind::NetworkUnreachable)];
    assert_eq!(classify(&other), Access::Unknown);
}

#[test]
fn the_target_is_another_address_of_a_private_24() {
    let target = |own: [u8; 4]| probe_target(Ipv4Addr::from(own)).map(|ip| ip.octets());
    assert_eq!(target([192, 168, 1, 23]), Some([192, 168, 1, 253]));
    assert_eq!(target([10, 0, 7, 253]), Some([10, 0, 7, 252]));
    assert_eq!(target([172, 20, 0, 5]), Some([172, 20, 0, 253]));
    // Public, Tailscale's shared range, loopback: not a local network.
    assert_eq!(target([8, 8, 8, 8]), None);
    assert_eq!(target([100, 95, 92, 27]), None);
    assert_eq!(target([127, 0, 0, 1]), None);
}

#[test]
fn a_probe_reports_each_send() {
    // Loopback is never a local network: both sends go.
    let sends = probe(Ipv4Addr::LOCALHOST);
    assert_eq!(sends.len(), 2);
    assert_eq!(classify(&sends), Access::NotDeclined);
}

#[test]
fn this_computer_has_an_ipv4_or_says_none() {
    // Whatever the machine's network, never an unspecified address.
    assert_ne!(own_ipv4(), Some(Ipv4Addr::UNSPECIFIED));
}

#[test]
fn other_systems_are_never_declined() {
    if !cfg!(target_os = "macos") {
        assert_eq!(SystemLocalNetwork.access(), Access::NotDeclined);
    }
}
