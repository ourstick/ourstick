use std::net::SocketAddr;

use rand::{Rng, SeedableRng};

use super::*;
use crate::record::ManifestHint;
use crate::stick_id::StickSigningKey;

fn ticket() -> Ticket {
    Ticket {
        stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
        node_id: NodeId([2; 32]),
        manifest_hint: ManifestHint {
            seq: 300,
            hash: [3; 32],
        },
        address_hints: vec![
            "192.168.1.20:4433".parse::<SocketAddr>().unwrap().into(),
            "[2001:db8::1]:4433".parse::<SocketAddr>().unwrap().into(),
            AddressHint::Relay {
                url: "https://relay.example.org./".to_owned(),
            },
        ],
        stick_key: StickKey::from_bytes([4; 32]),
    }
}

fn link() -> String {
    ticket().to_link().unwrap()
}

fn split(link: &str) -> (&str, &str) {
    link.strip_prefix(LINK_PREFIX)
        .unwrap()
        .split_once('.')
        .unwrap()
}

#[test]
fn link_roundtrips() {
    assert_eq!(link().parse::<Ticket>().unwrap(), ticket());
}

#[test]
fn link_roundtrips_without_hints() {
    let ticket = Ticket {
        address_hints: Vec::new(),
        ..ticket()
    };
    assert_eq!(ticket.to_link().unwrap().parse::<Ticket>().unwrap(), ticket);
}

#[test]
fn link_has_expected_shape() {
    let link = link();
    let (public, secret) = split(&link);
    let lowercase_base32 = |text: &str| {
        text.chars()
            .all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c))
    };
    assert!(lowercase_base32(public), "{public}");
    assert!(lowercase_base32(secret), "{secret}");
    // 36 secret bytes are 58 base32 characters.
    assert_eq!(secret.len(), 58);
}

#[test]
fn link_is_deterministic() {
    assert_eq!(link(), link());
}

#[test]
fn parse_accepts_bare_form_uppercase_and_whitespace() {
    let link = link();
    let bare = link.strip_prefix(LINK_PREFIX).unwrap();
    assert_eq!(bare.parse::<Ticket>().unwrap(), ticket());
    assert_eq!(link.to_uppercase().parse::<Ticket>().unwrap(), ticket());
    assert_eq!(format!("  {link}\n").parse::<Ticket>().unwrap(), ticket());
}

#[test]
fn debug_hides_key() {
    let printed = format!("{:?}", ticket());
    assert!(printed.contains("<redacted>"), "{printed}");
    let key_hex = data_encoding::HEXLOWER.encode(ticket().stick_key.as_bytes());
    assert!(!printed.contains(&key_hex), "{printed}");
    assert!(!printed.contains("[4, 4, 4"), "{printed}");
}

#[test]
fn parse_reports_not_a_ticket() {
    for text in [
        "",
        "hello",
        "#abc",
        "https://example.com/s/#abc.def",
        "https://ourstick.com/",
        "notes.txt",
        "example.com",
        "a.b",
    ] {
        assert_eq!(
            text.parse::<Ticket>(),
            Err(TicketError::NotATicket),
            "{text:?}"
        );
    }
}

#[test]
fn parse_reports_missing_secret() {
    let link = link();
    // The last dot: "ourstick.com" has one too.
    let (without_secret, _) = link.rsplit_once('.').unwrap();
    for text in [without_secret.to_owned(), format!("{without_secret}.")] {
        assert_eq!(
            text.parse::<Ticket>(),
            Err(TicketError::MissingSecret),
            "{text}"
        );
    }
}

#[test]
fn parse_reports_damaged_public_part() {
    let link = link();
    let (public, secret) = split(&link);

    let truncated = format!("{}.{secret}", &public[..public.len() - 8]);
    assert_eq!(
        truncated.parse::<Ticket>(),
        Err(TicketError::PublicPartDamaged)
    );

    let mut changed: Vec<char> = public.chars().collect();
    changed[10] = if changed[10] == 'a' { 'b' } else { 'a' };
    let changed = format!("{}.{secret}", changed.iter().collect::<String>());
    assert_eq!(
        changed.parse::<Ticket>(),
        Err(TicketError::PublicPartDamaged)
    );

    // Not base32: damaged if it carries our prefix, otherwise not a ticket at all.
    assert_eq!(
        format!("{LINK_PREFIX}not-base32!.{secret}").parse::<Ticket>(),
        Err(TicketError::PublicPartDamaged)
    );
    assert_eq!(
        format!("not-base32!.{secret}").parse::<Ticket>(),
        Err(TicketError::NotATicket)
    );
}

#[test]
fn parse_reports_damaged_secret_part() {
    let link = link();
    let (public, secret) = split(&link);

    let truncated = format!("{public}.{}", &secret[..secret.len() - 1]);
    let mut changed: Vec<char> = secret.chars().collect();
    changed[0] = if changed[0] == 'a' { 'b' } else { 'a' };
    let changed = format!("{public}.{}", changed.iter().collect::<String>());
    for text in [truncated, changed] {
        assert_eq!(
            text.parse::<Ticket>(),
            Err(TicketError::SecretPartDamaged),
            "{text}"
        );
    }
}

#[test]
fn parse_rejects_secret_from_another_ticket() {
    let other = Ticket {
        manifest_hint: ManifestHint {
            seq: 300,
            hash: [9; 32],
        },
        ..ticket()
    };
    let other_link = other.to_link().unwrap();
    let (_, other_secret) = split(&other_link);
    let link = link();
    let (public, _) = split(&link);
    assert_eq!(
        format!("{public}.{other_secret}").parse::<Ticket>(),
        Err(TicketError::SecretPartDamaged)
    );
}

/// Builds a public part with a valid check around an arbitrary version and body.
fn public_with(version: u8, body: &[u8]) -> Vec<u8> {
    let mut public = vec![version];
    public.extend(body);
    let hash = blake3::hash(&public);
    public.extend(&hash.as_bytes()[..4]);
    public
}

fn link_from_public(public: &[u8]) -> String {
    let key = [4u8; 32];
    let mut hasher = blake3::Hasher::new();
    hasher.update(public);
    hasher.update(&key);
    let secret = [&key[..], &hasher.finalize().as_bytes()[..4]].concat();
    let encode = |bytes: &[u8]| {
        data_encoding::BASE32_NOPAD
            .encode(bytes)
            .to_ascii_lowercase()
    };
    format!("{LINK_PREFIX}{}.{}", encode(public), encode(&secret))
}

#[test]
fn parse_reports_newer_version() {
    let link = link_from_public(&public_with(2, b"anything"));
    assert_eq!(
        link.parse::<Ticket>(),
        Err(TicketError::UnsupportedVersion(2))
    );
}

#[test]
fn parse_rejects_invalid_stick_id() {
    let mut body = vec![0u8; 32];
    body[0] = 2; // y = 2 is not on the curve
    body.extend([0u8; 32]); // node id
    body.push(0); // manifest seq
    body.extend([0u8; 32]); // manifest hash
    body.push(0); // no hints
    let link = link_from_public(&public_with(TICKET_FORMAT_VERSION, &body));
    assert_eq!(link.parse::<Ticket>(), Err(TicketError::InvalidStickId));
}

#[test]
fn parse_rejects_trailing_bytes_in_body() {
    let link = link();
    let (public, _) = split(&link);
    let public = data_encoding::BASE32_NOPAD
        .decode(public.to_ascii_uppercase().as_bytes())
        .unwrap();
    let body = &public[1..public.len() - 4];
    let with_trailing = [body, &[0]].concat();
    let link = link_from_public(&public_with(TICKET_FORMAT_VERSION, &with_trailing));
    assert_eq!(link.parse::<Ticket>(), Err(TicketError::PublicPartDamaged));
}

#[test]
fn writer_and_reader_enforce_hint_rules() {
    let too_many = Ticket {
        address_hints: vec![AddressHint::Relay { url: "a".into() }; MAX_ADDRESS_HINTS + 1],
        ..ticket()
    };
    assert_eq!(
        too_many.to_link(),
        Err(TicketError::TooManyAddressHints(MAX_ADDRESS_HINTS + 1))
    );

    for url in [
        String::new(),
        "https://relay.example.org/\u{1b}[31m".to_owned(),
        "has space".to_owned(),
        "x".repeat(MAX_RELAY_URL_LEN + 1),
    ] {
        let ticket = Ticket {
            address_hints: vec![AddressHint::Relay { url: url.clone() }],
            ..ticket()
        };
        assert_eq!(
            ticket.to_link(),
            Err(TicketError::InvalidRelayUrl),
            "{url:?}"
        );
    }

    // A reader rejects the same thing when a writer did not check.
    let body = postcard::to_allocvec(&(
        ticket().stick_id.to_bytes(),
        [2u8; 32],
        300u64,
        [3u8; 32],
        vec![AddressHint::Relay { url: "a b".into() }],
    ))
    .unwrap();
    let link = link_from_public(&public_with(TICKET_FORMAT_VERSION, &body));
    assert_eq!(link.parse::<Ticket>(), Err(TicketError::InvalidRelayUrl));
}

#[test]
fn parse_rejects_very_long_input_early() {
    let text = format!("{LINK_PREFIX}{}.abc", "a".repeat(5000));
    assert_eq!(text.parse::<Ticket>(), Err(TicketError::TooLong));
}

#[test]
fn address_hint_from_socket_addr() {
    let v4: AddressHint = "10.0.0.1:80".parse::<SocketAddr>().unwrap().into();
    assert_eq!(
        v4,
        AddressHint::Ipv4 {
            ip: [10, 0, 0, 1],
            port: 80
        }
    );
    let v6: AddressHint = "[::1]:443".parse::<SocketAddr>().unwrap().into();
    let mut loopback = [0u8; 16];
    loopback[15] = 1;
    assert_eq!(
        v6,
        AddressHint::Ipv6 {
            ip: loopback,
            port: 443
        }
    );
}

#[test]
fn parse_never_panics_on_random_or_mutated_input() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x0071_C4E7);
    let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyz234567#/:.ABZ019 é"
        .chars()
        .collect();
    for _ in 0..3_000 {
        let length = rng.gen_range(0..300);
        let text: String = (0..length)
            .map(|_| alphabet[rng.gen_range(0..alphabet.len())])
            .collect();
        let _ = text.parse::<Ticket>();
        let _ = format!("{LINK_PREFIX}{text}").parse::<Ticket>();
    }

    let link: Vec<char> = link().chars().collect();
    for _ in 0..3_000 {
        let mut mutated = link.clone();
        let index = rng.gen_range(LINK_PREFIX.len()..mutated.len());
        mutated[index] = alphabet[rng.gen_range(0..alphabet.len())];
        let mutated: String = mutated.into_iter().collect();
        if let Ok(parsed) = mutated.parse::<Ticket>() {
            // A mutation that still parses must be a no-op (e.g. changed case).
            assert_eq!(parsed, ticket(), "{mutated}");
        }
    }
}

#[test]
fn manifest_seq_roundtrips_at_the_extremes() {
    for seq in [0, 1, 127, 128, u64::MAX] {
        let ticket = Ticket {
            manifest_hint: ManifestHint { seq, hash: [3; 32] },
            ..ticket()
        };
        let parsed: Ticket = ticket.to_link().unwrap().parse().unwrap();
        assert_eq!(parsed.manifest_hint.seq, seq);
    }
}

mod link_form;
