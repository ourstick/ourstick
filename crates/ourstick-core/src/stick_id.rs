//! The identity of a stick: an Ed25519 key pair, separate from the iroh node key.
//!
//! The verifying key is the [`StickId`], shared in every ticket. The
//! [`StickSigningKey`] stays with the owner and signs manifest records.
//! See `docs/decisions/0004-signed-manifest-records.md`.

use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;

/// The public identity of a stick: its Ed25519 verifying key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StickId(VerifyingKey);

/// The bytes are not a valid Ed25519 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a valid Ed25519 public key")]
pub struct InvalidPublicKey;

impl StickId {
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, InvalidPublicKey> {
        VerifyingKey::from_bytes(bytes)
            .map(Self)
            .map_err(|_| InvalidPublicKey)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// Strict Ed25519 verification (RFC 8032 checks, no small-order keys).
    pub(crate) fn verify_strict(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        let signature = Signature::from_bytes(signature);
        self.0.verify_strict(message, &signature).is_ok()
    }
}

impl fmt::Debug for StickId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = data_encoding::HEXLOWER.encode(&self.to_bytes());
        write!(formatter, "StickId({hex})")
    }
}

/// The owner's secret for a stick. It never leaves the owner's device, and must
/// never be regenerated once a ticket has been shared.
///
/// `Debug` never prints the key.
#[derive(Clone)]
pub struct StickSigningKey(SigningKey);

impl StickSigningKey {
    /// Creates a new key pair from the operating system's secure RNG.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        Self::from_bytes(&seed)
    }

    /// Restores a key from its 32-byte seed.
    pub fn from_bytes(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    /// The 32-byte seed, for storing the key. Treat it as a secret.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn stick_id(&self) -> StickId {
        StickId(self.0.verifying_key())
    }

    pub(crate) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

impl fmt::Debug for StickSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "StickSigningKey({:?}, <redacted>)",
            self.stick_id()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_differ() {
        let first = StickSigningKey::generate();
        let second = StickSigningKey::generate();
        assert_ne!(first.stick_id(), second.stick_id());
    }

    #[test]
    fn signing_key_roundtrips_through_bytes() {
        let key = StickSigningKey::generate();
        let restored = StickSigningKey::from_bytes(&key.to_bytes());
        assert_eq!(restored.stick_id(), key.stick_id());
    }

    #[test]
    fn stick_id_roundtrips_through_bytes() {
        let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
        assert_eq!(StickId::from_bytes(&stick_id.to_bytes()), Ok(stick_id));
    }

    #[test]
    fn stick_id_rejects_invalid_point() {
        // y = 2 is not on the curve.
        let mut bytes = [0u8; 32];
        bytes[0] = 2;
        assert_eq!(StickId::from_bytes(&bytes), Err(InvalidPublicKey));
    }

    #[test]
    fn signature_verifies_only_for_same_message_and_key() {
        let key = StickSigningKey::from_bytes(&[1; 32]);
        let signature = key.sign(b"message");
        assert!(key.stick_id().verify_strict(b"message", &signature));
        assert!(!key.stick_id().verify_strict(b"other", &signature));
        let other = StickSigningKey::from_bytes(&[2; 32]).stick_id();
        assert!(!other.verify_strict(b"message", &signature));
    }

    #[test]
    fn debug_prints_public_key_but_not_secret() {
        let key = StickSigningKey::from_bytes(&[0xAB; 32]);
        let printed = format!("{key:?}");
        assert!(printed.contains("<redacted>"), "{printed}");
        assert!(!printed.contains(&"ab".repeat(32)), "{printed}");
        assert!(printed.contains(&data_encoding::HEXLOWER.encode(&key.stick_id().to_bytes())));
    }
}
