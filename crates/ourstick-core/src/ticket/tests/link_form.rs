//! The link's text form: where the ticket is in it (ADR 0022), and what
//! around it is not part of it.

use super::*;

#[test]
fn the_whole_ticket_is_in_the_fragment() {
    // What a browser sends to the site: our address, nothing of the ticket
    // (neither the key nor who and where the sender is).
    let link = link();
    let (before_fragment, _) = link.split_once('#').unwrap();
    assert_eq!(before_fragment, "https://ourstick.com/s/");

    let (public, _) = split(&link);
    let public = data_encoding::BASE32_NOPAD
        .decode(public.to_ascii_uppercase().as_bytes())
        .unwrap();
    let key = ticket().stick_key;
    assert!(!public.windows(32).any(|window| window == key.as_bytes()));
}

#[test]
fn a_link_cut_before_its_fragment_is_damaged() {
    // Our site's address with the part after "#" lost, or a link in the
    // form before 2026-10-09 (no longer read: nobody had one).
    let link = link();
    let (public, secret) = split(&link);
    for text in [
        "https://ourstick.com/s/".to_owned(),
        "https://ourstick.com/s/#".to_owned(),
        format!("https://ourstick.com/s/{public}#{secret}"),
    ] {
        assert_eq!(
            text.parse::<Ticket>(),
            Err(TicketError::PublicPartDamaged),
            "{text}"
        );
    }
}

#[test]
fn punctuation_around_a_link_is_not_part_of_it() {
    let link = link();
    for text in [
        format!("{link}."),
        format!("({link})"),
        format!("<{link}>"),
        format!("\"{link}\","),
        format!("{link}!)"),
    ] {
        assert_eq!(text.parse::<Ticket>().unwrap(), ticket(), "{text}");
    }
}
