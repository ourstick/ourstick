//! `version.json` (ADR 0033 rules 1, 3 and 4): the notice and its signature
//! in one file, so that a cache never serves one without the other. The
//! signature is checked before the notice is parsed.
//!
//! ```json
//! { "notice": "<the notice, as the text that was signed>",
//!   "signature": "<the .minisig file>" }
//! ```
//!
//! A notice of the release key:
//! `{"serial": 3, "made": "2026-11-20", "windows": {"newest": "0.3.0",
//! "security": "0.3.0"}, "macos": {…}, "linux": {…}}`. One of the recovery
//! key says only that the release key changed:
//! `{"serial": 1, "made": "…", "new_release_key": "<its id, 16 hex>"}`.
//! Unknown fields are allowed (a later version may add some).

use semver::Version;
use serde::Deserialize;

use super::keys::NoticeKeys;
use super::signature::{check, SignatureError, Signer};

/// The systems a notice names: the `.exe`, the `.dmg` and the amd64 `.deb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    Windows,
    Macos,
    Linux,
}

impl System {
    /// This build's system; None on the phones, which their stores update.
    pub fn this() -> Option<Self> {
        if cfg!(any(target_os = "android", target_os = "ios")) {
            None
        } else if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// What a good notice says to this system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Of the release key: the newest version, and the oldest that is safe.
    Versions {
        serial: u64,
        key_id: String,
        newest: Version,
        security: Version,
    },
    /// Of the recovery key: the release key is now the one with this id.
    ReleaseKeyChanged {
        serial: u64,
        key_id: String,
        new_release_key: String,
    },
}

impl Notice {
    pub fn serial(&self) -> u64 {
        match self {
            Self::Versions { serial, .. } | Self::ReleaseKeyChanged { serial, .. } => *serial,
        }
    }

    /// The id of the key that signed it: serials are counted per key.
    pub fn key_id(&self) -> &str {
        match self {
            Self::Versions { key_id, .. } | Self::ReleaseKeyChanged { key_id, .. } => key_id,
        }
    }
}

/// Why a file is not a notice for this system. Never holds the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoticeError {
    #[error("not a version.json")]
    NotTheFile,
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error("a signed notice that does not hold: {0}")]
    Unusable(&'static str),
}

#[derive(Deserialize)]
struct File {
    notice: String,
    signature: String,
}

#[derive(Deserialize)]
struct Fields {
    serial: u64,
    made: String,
    windows: Option<ForSystem>,
    macos: Option<ForSystem>,
    linux: Option<ForSystem>,
    new_release_key: Option<String>,
}

#[derive(Deserialize)]
struct ForSystem {
    newest: String,
    security: String,
}

/// Checks and reads `version.json` for `system`.
pub fn read_notice(bytes: &[u8], keys: &NoticeKeys, system: System) -> Result<Notice, NoticeError> {
    let file: File = serde_json::from_slice(bytes).map_err(|_| NoticeError::NotTheFile)?;
    let signer = check(
        file.notice.as_bytes(),
        &file.signature,
        &keys.release,
        &keys.recovery,
    )?;
    let fields: Fields = serde_json::from_str(&file.notice)
        .map_err(|_| NoticeError::Unusable("not a notice's fields"))?;
    let serial = fields.serial;
    if i64::try_from(serial).is_err() {
        return Err(NoticeError::Unusable("a serial too large"));
    }
    if !is_day(&fields.made) {
        return Err(NoticeError::Unusable("made is not a day"));
    }
    match signer {
        Signer::Recovery => {
            let new_release_key = fields
                .new_release_key
                .filter(|id| id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .ok_or(NoticeError::Unusable("the recovery key says no new key"))?
                .to_ascii_uppercase();
            Ok(Notice::ReleaseKeyChanged {
                serial,
                key_id: keys.recovery.id_hex(),
                new_release_key,
            })
        }
        Signer::Release => {
            if fields.new_release_key.is_some() {
                return Err(NoticeError::Unusable(
                    "only the recovery key names a new key",
                ));
            }
            let for_system = match system {
                System::Windows => fields.windows,
                System::Macos => fields.macos,
                System::Linux => fields.linux,
            }
            .ok_or(NoticeError::Unusable("nothing for this system"))?;
            let newest = plain_version(&for_system.newest)?;
            let security = plain_version(&for_system.security)?;
            if security > newest {
                return Err(NoticeError::Unusable("security above newest"));
            }
            Ok(Notice::Versions {
                serial,
                key_id: keys.release.id_hex(),
                newest,
                security,
            })
        }
    }
}

/// `X.Y.Z` and nothing else: no pre-release, no build.
pub fn plain_version(text: &str) -> Result<Version, NoticeError> {
    Version::parse(text)
        .ok()
        .filter(|version| version.pre.is_empty() && version.build.is_empty())
        .ok_or(NoticeError::Unusable("a version that is not X.Y.Z"))
}

/// `YYYY-MM-DD`, in digits.
fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "notice_tests.rs"]
mod tests;
