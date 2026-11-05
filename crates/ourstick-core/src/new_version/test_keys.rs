//! Tests only: keys that sign notices as minisign does (`-l`, the legacy
//! kind), so that no test needs the real keys.

use data_encoding::BASE64;
use ed25519_dalek::{Signer as _, SigningKey};

use super::keys::{NoticeKey, NoticeKeys};

pub(super) struct TestKey {
    signing: SigningKey,
    id: [u8; 8],
}

impl TestKey {
    pub(super) fn new(seed: u8) -> Self {
        Self {
            signing: SigningKey::from_bytes(&[seed; 32]),
            id: [seed, 1, 2, 3, 4, 5, 6, 7],
        }
    }

    pub(super) fn public(&self) -> NoticeKey {
        NoticeKey::from_parts(self.id, self.signing.verifying_key())
    }

    /// A `.minisig` text over `bytes`, as `minisign -S -l` writes it.
    pub(super) fn sign(&self, bytes: &[u8]) -> String {
        self.sign_as(b"Ed", bytes, "OurStick test")
    }

    pub(super) fn sign_as(&self, kind: &[u8; 2], bytes: &[u8], comment: &str) -> String {
        let signature = self.signing.sign(bytes).to_bytes();
        let mut main = kind.to_vec();
        main.extend_from_slice(&self.id);
        main.extend_from_slice(&signature);
        let mut both = signature.to_vec();
        both.extend_from_slice(comment.as_bytes());
        let global = self.signing.sign(&both).to_bytes();
        format!(
            "untrusted comment: signature from minisign secret key\n{}\ntrusted comment: {comment}\n{}\n",
            BASE64.encode(&main),
            BASE64.encode(&global)
        )
    }
}

/// A release key (seed 1) and a recovery key (seed 2).
pub(super) fn test_keys() -> (TestKey, TestKey, NoticeKeys) {
    let release = TestKey::new(1);
    let recovery = TestKey::new(2);
    let keys = NoticeKeys {
        release: release.public(),
        recovery: recovery.public(),
    };
    (release, recovery, keys)
}

/// A whole `version.json`: the notice and its signature together.
pub(super) fn version_json(key: &TestKey, notice: &str) -> Vec<u8> {
    serde_json::json!({ "notice": notice, "signature": key.sign(notice.as_bytes()) })
        .to_string()
        .into_bytes()
}
