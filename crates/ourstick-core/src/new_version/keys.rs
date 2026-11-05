//! The public keys a notice is checked against (ADR 0033, "Keys"): read from
//! the repository's `release-keys/`, the one place they are kept.

use data_encoding::BASE64;
use ed25519_dalek::VerifyingKey;

/// A minisign public key: its id and its Ed25519 key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeKey {
    id: [u8; 8],
    key: VerifyingKey,
}

impl NoticeKey {
    /// Reads a minisign public key file (or its one base64 line). None if it
    /// is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let line = text.lines().map(str::trim).rfind(|line| !line.is_empty())?;
        let bytes = BASE64.decode(line.as_bytes()).ok()?;
        if bytes.len() != 42 || &bytes[..2] != b"Ed" {
            return None;
        }
        let id = bytes[2..10].try_into().ok()?;
        let key = VerifyingKey::from_bytes(&bytes[10..42].try_into().ok()?).ok()?;
        Some(Self { id, key })
    }

    #[cfg(test)]
    pub(super) fn from_parts(id: [u8; 8], key: VerifyingKey) -> Self {
        Self { id, key }
    }

    pub(super) fn id(&self) -> [u8; 8] {
        self.id
    }

    pub(super) fn key(&self) -> &VerifyingKey {
        &self.key
    }

    /// The id as minisign shows it (`2DDEFEB70EAC0BBB`): the bytes from last
    /// to first, in hex.
    pub fn id_hex(&self) -> String {
        id_hex(&self.id)
    }
}

pub(super) fn id_hex(id: &[u8; 8]) -> String {
    id.iter().rev().map(|byte| format!("{byte:02X}")).collect()
}

/// The keys a notice may be signed with: the release key, and the recovery
/// key that may only say the release key changed.
#[derive(Debug, Clone)]
pub struct NoticeKeys {
    pub release: NoticeKey,
    pub recovery: NoticeKey,
}

impl NoticeKeys {
    /// OurStick's own keys, built in.
    pub fn ourstick() -> Self {
        Self {
            release: NoticeKey::parse(include_str!(
                "../../../../release-keys/ourstick-release.pub"
            ))
            .expect("release-keys/ourstick-release.pub is a minisign public key"),
            recovery: NoticeKey::parse(include_str!(
                "../../../../release-keys/ourstick-recovery.pub"
            ))
            .expect("release-keys/ourstick-recovery.pub is a minisign public key"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_keys_are_the_ones_made_on_2026_11_05() {
        let keys = NoticeKeys::ourstick();
        assert_eq!(keys.release.id_hex(), "2DDEFEB70EAC0BBB");
        assert_eq!(keys.recovery.id_hex(), "971E8F0C5AA6A65E");
    }

    #[test]
    fn what_is_not_a_key_is_refused() {
        for text in [
            "",
            "untrusted comment: nothing",
            "RWS7C6wOt/7eLZ1c4gP+CYXHWKYycOKldo6+LFUSHsA5TUwdiBot58",
            "not base64 at all!",
            // A signature line, not a key.
            "RWS7C6wOt/7eLZ1c4gP+CYXHWKYycOKldo6+LFUSHsA5TUwdiBot58YhAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        ] {
            assert!(NoticeKey::parse(text).is_none(), "{text}");
        }
    }
}
