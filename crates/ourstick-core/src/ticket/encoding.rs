//! Byte and text encoding of a ticket. See `docs/protocol.md`.

use std::str::FromStr;

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{AddressHint, NodeId, Ticket, TicketError, MAX_PUBLIC_PART_LEN};
use crate::cipher::StickKey;
use crate::record::ManifestHint;
use crate::stick_id::StickId;

/// Version of the ticket format written by [`Ticket::to_link`].
pub const TICKET_FORMAT_VERSION: u8 = 1;

/// Everything before the public part in the link form of a ticket. Both
/// parts are in the fragment, which a browser never sends: the site that
/// serves the page sees neither the key nor who and where the sender is.
pub const LINK_PREFIX: &str = "https://ourstick.com/s/#";

/// Between the public part and the secret part.
const SEPARATOR: char = '.';

const CHECK_LEN: usize = 4;
const SECRET_PART_LEN: usize = 32 + CHECK_LEN;

/// Longest text accepted before decoding anything, so hostile input is cheap to reject.
const MAX_TEXT_LEN: usize = 4096;

/// The shortest public part, in base32 characters: version, stick id, node
/// id, a one-byte seq, manifest hash, no hints, check (102 bytes). Shorter
/// text without our prefix ("notes.txt") is not taken for a damaged ticket.
const MIN_PUBLIC_TEXT_LEN: usize = 164;

/// What a chat app or a sentence may put around a link: never part of one
/// (base32 and the prefix neither start nor end with them).
const AROUND_A_LINK: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '(', '[', '{', '<', '"', '\'', '`',
];

/// The postcard-encoded body of the public part. Field order is the wire order.
#[derive(Serialize, Deserialize)]
struct PublicBody {
    stick_id: [u8; 32],
    node_id: [u8; 32],
    manifest_seq: u64,
    manifest_hash: [u8; 32],
    address_hints: Vec<AddressHint>,
}

fn check(parts: &[&[u8]]) -> [u8; CHECK_LEN] {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    let hash = hasher.finalize();
    hash.as_bytes()[..CHECK_LEN].try_into().expect("4 bytes")
}

fn encode_base32(bytes: &[u8]) -> String {
    BASE32_NOPAD.encode(bytes).to_ascii_lowercase()
}

/// The upper-case copy is wiped: it may be a secret part.
fn decode_base32(text: &str) -> Option<Vec<u8>> {
    let upper = Zeroizing::new(text.to_ascii_uppercase());
    BASE32_NOPAD.decode(upper.as_bytes()).ok()
}

impl Ticket {
    /// The ticket as a link: `https://ourstick.com/s/#<public>.<secret>`.
    ///
    /// The result grants read access to the stick. Never log it.
    pub fn to_link(&self) -> Result<String, TicketError> {
        let public = self.public_part()?;
        let secret = self.secret_part(&public);
        Ok(format!(
            "{LINK_PREFIX}{}{SEPARATOR}{}",
            encode_base32(&public),
            encode_base32(&secret)
        ))
    }

    fn public_part(&self) -> Result<Vec<u8>, TicketError> {
        self.validate()?;
        let body = PublicBody {
            stick_id: self.stick_id.to_bytes(),
            node_id: self.node_id.0,
            manifest_seq: self.manifest_hint.seq,
            manifest_hash: self.manifest_hint.hash,
            address_hints: self.address_hints.clone(),
        };
        let mut public = vec![TICKET_FORMAT_VERSION];
        public.extend(postcard::to_allocvec(&body).expect("encoding to a Vec cannot fail"));
        let check = check(&[&public]);
        public.extend(check);
        if public.len() > MAX_PUBLIC_PART_LEN {
            return Err(TicketError::PublicPartTooLong(public.len()));
        }
        Ok(public)
    }

    /// The key and its check, wiped when dropped.
    fn secret_part(&self, public: &[u8]) -> Zeroizing<Vec<u8>> {
        let key = self.stick_key.as_bytes();
        Zeroizing::new([&key[..], &check(&[public, key])].concat())
    }

    fn from_parts(public_text: &str, secret_text: &str) -> Result<Self, TicketError> {
        let public = decode_base32(public_text).ok_or(TicketError::PublicPartDamaged)?;
        if public.len() > MAX_PUBLIC_PART_LEN {
            return Err(TicketError::PublicPartTooLong(public.len()));
        }
        let Some((body_with_version, public_check)) = public.split_last_chunk::<CHECK_LEN>() else {
            return Err(TicketError::PublicPartDamaged);
        };
        if check(&[body_with_version]) != *public_check {
            return Err(TicketError::PublicPartDamaged);
        }
        let (&version, body) = body_with_version
            .split_first()
            .ok_or(TicketError::PublicPartDamaged)?;
        if version != TICKET_FORMAT_VERSION {
            return Err(TicketError::UnsupportedVersion(version));
        }
        // The check passed, so a body that does not decode was written wrongly,
        // not damaged in transit. Report it as damaged anyway: the user can only
        // ask for the ticket again.
        let (body, rest): (PublicBody, _) =
            postcard::take_from_bytes(body).map_err(|_| TicketError::PublicPartDamaged)?;
        if !rest.is_empty() {
            return Err(TicketError::PublicPartDamaged);
        }

        let secret =
            Zeroizing::new(decode_base32(secret_text).ok_or(TicketError::SecretPartDamaged)?);
        let secret: &[u8; SECRET_PART_LEN] = secret
            .as_slice()
            .try_into()
            .map_err(|_| TicketError::SecretPartDamaged)?;
        let (key, secret_check) = secret.split_at(32);
        if check(&[&public, key]) != secret_check {
            return Err(TicketError::SecretPartDamaged);
        }

        let ticket = Self {
            stick_id: StickId::from_bytes(&body.stick_id)
                .map_err(|_| TicketError::InvalidStickId)?,
            node_id: NodeId(body.node_id),
            manifest_hint: ManifestHint {
                seq: body.manifest_seq,
                hash: body.manifest_hash,
            },
            address_hints: body.address_hints,
            stick_key: StickKey::from_bytes(key.try_into().expect("32 bytes")),
        };
        ticket.validate()?;
        Ok(ticket)
    }
}

impl FromStr for Ticket {
    type Err = TicketError;

    /// Reads a ticket in link form, or in the bare form `<public>.<secret>`.
    /// Surrounding whitespace and punctuation ("<link>.", "(link)") are
    /// ignored; letters may be in either case.
    fn from_str(text: &str) -> Result<Self, TicketError> {
        let text = text.trim().trim_matches(AROUND_A_LINK).trim();
        if text.len() > MAX_TEXT_LEN {
            return Err(TicketError::TooLong);
        }
        // Our site's address with less after it than a link has (the part
        // after "#" lost on the way) is a link cut short, not another text.
        let site = &LINK_PREFIX[..LINK_PREFIX.len() - 1];
        let starts_with = |start: &str| {
            text.get(..start.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(start))
        };
        let has_prefix = starts_with(LINK_PREFIX);
        if !has_prefix && starts_with(site) {
            return Err(TicketError::PublicPartDamaged);
        }
        let rest = if has_prefix {
            &text[LINK_PREFIX.len()..]
        } else {
            text
        };
        // Without our prefix, only something that looks like base32 counts as a
        // (possibly damaged) ticket; anything else, like another site's link, is not one.
        let looks_like_base32 = |part: &str| {
            part.bytes()
                .all(|byte| byte.is_ascii_alphabetic() || (b'2'..=b'7').contains(&byte))
        };
        let bare_ticket = rest.split_once(SEPARATOR).is_some_and(|(public, secret)| {
            public.len() >= MIN_PUBLIC_TEXT_LEN
                && looks_like_base32(public)
                && looks_like_base32(secret)
        });
        if !has_prefix && !bare_ticket {
            return Err(TicketError::NotATicket);
        }
        match rest.split_once(SEPARATOR) {
            Some((public, secret)) if !public.is_empty() && !secret.is_empty() => {
                Self::from_parts(public, secret)
            }
            Some((public, _)) if !public.is_empty() => Err(TicketError::MissingSecret),
            None if has_prefix && !rest.is_empty() => Err(TicketError::MissingSecret),
            _ if has_prefix => Err(TicketError::PublicPartDamaged),
            _ => Err(TicketError::NotATicket),
        }
    }
}
