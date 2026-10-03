//! A reader's own key for one received stick (ADR 0015, rule 2): made once
//! when the stick is received, kept in the secret store, deleted when the
//! stick is forgotten. Its public key is the reader id.

use std::fmt;

use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;

/// The reader's signing key for one stick. `Debug` never prints it.
#[derive(Clone)]
pub struct ReaderKey(SigningKey);

impl ReaderKey {
    /// A new key from the operating system's secure RNG.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        Self::from_bytes(&seed)
    }

    /// Restores a key from its 32-byte seed.
    pub fn from_bytes(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    /// The 32-byte seed, for the secret store. Treat it as a secret.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn reader_id(&self) -> ReaderId {
        ReaderId(self.0.verifying_key().to_bytes())
    }

    pub(super) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

impl fmt::Debug for ReaderKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ReaderKey({:?}, <redacted>)", self.reader_id())
    }
}

/// Who a report is from: the public key of a reader's key for the stick.
/// Names one computer's copy of one stick, nothing more.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReaderId([u8; 32]);

impl ReaderId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for ReaderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short: String = self.0[..4]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        write!(formatter, "ReaderId({short}…)")
    }
}
