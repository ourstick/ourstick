//! How far a reader is, told to the owner (ADR 0015): a report signed with
//! the reader's own key for the stick and sealed with a key derived from the
//! StickKey, so only someone with the link can send or read one, and each
//! reader speaks only for itself. See `docs/protocol.md`.
//!
//! Sealed form: version (1 byte), nonce (24), XChaCha20-Poly1305 ciphertext
//! of `reader id (32) ‖ signature (64) ‖ postcard body`, with the version
//! and the StickId as associated data. The signature covers
//! [`SIGNATURE_CONTEXT`], the version, the StickId and the body.

mod reader_key;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::cipher::StickKey;
use crate::stick_id::StickId;

pub use reader_key::{ReaderId, ReaderKey};

/// The most bytes a report takes on the wire, the StickId before it
/// included.
pub const MAX_REPORT_BYTES: usize = 512;

/// The most bytes of a sealed report: what is left after the StickId.
pub const MAX_SEALED_BYTES: usize = MAX_REPORT_BYTES - 32;

/// The most characters of a name a receiver typed.
pub const MAX_NAME_CHARS: usize = 40;

const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const SIGNATURE_LEN: usize = 64;

/// BLAKE3 `derive_key` context for the key reports are sealed with.
const KEY_CONTEXT: &str = "OurStick 2026-10-04 report XChaCha20-Poly1305 key";

/// What a reader's signature covers before the StickId and the body: a
/// report's signature means nothing anywhere else.
pub const SIGNATURE_CONTEXT: &[u8] = b"OurStick 2026-10-04 report signature";

/// Why a reader waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitReason {
    SenderAway,
    NoRoom,
    FolderMissing,
    Other,
}

/// Where a reader is with the version it reports on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportState {
    /// A whole percent (0–99) of what it gets.
    Downloading {
        percent: u8,
    },
    UpToDate,
    Waiting(WaitReason),
    Paused,
    /// The receiver forgot the stick: its line goes.
    Forgotten,
}

/// What kind of computer the reader is: told without asking (every download
/// from a website tells it too), never the computer's own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerKind {
    MacOs,
    Windows,
    Linux,
    Other,
}

impl ComputerKind {
    /// This computer's kind.
    pub fn this() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// What a reader tells the owner (ADR 0015, rule 1): no bytes, no file
/// counts, no paths, so the owner learns *that* something is left out,
/// never what or how much.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderReport {
    /// Grows with every report of this reader, across restarts.
    pub counter: u64,
    /// The version (record `seq`) it is about.
    pub seq: u64,
    pub state: ReportState,
    /// It gets the whole stick: nothing left out.
    pub everything: bool,
    pub kind: ComputerKind,
    /// Only one the receiver typed (at most [`MAX_NAME_CHARS`], none of the
    /// characters a manifest's names may not have).
    pub name: Option<String>,
}

/// A report opened by the owner: who sent it, and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub reader: ReaderId,
    pub report: ReaderReport,
}

/// Why a report was not sent, or not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("a report is longer than {MAX_REPORT_BYTES} bytes")]
    TooBig,
    #[error("a report is cut short or not well formed")]
    Malformed,
    #[error("a report of an unknown version")]
    UnknownVersion,
    #[error("a report does not open with this stick's key")]
    Sealed,
    #[error("a report's signature does not hold")]
    BadSignature,
    #[error("a report's name is too long or has characters a name may not have")]
    BadName,
}

/// Seals `report` for the owner of `stick_id`, signed with `reader_key`.
pub fn seal(
    report: &ReaderReport,
    reader_key: &ReaderKey,
    stick_id: &StickId,
    stick_key: &StickKey,
) -> Result<Vec<u8>, ReportError> {
    check(report)?;
    let body = postcard::to_allocvec(report).map_err(|_| ReportError::Malformed)?;
    let signature = reader_key.sign(&signed_message(stick_id, &body));
    let plain = [&reader_key.reader_id().to_bytes()[..], &signature, &body].concat();
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let sealed = aead(stick_key)
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &associated(stick_id),
            },
        )
        .map_err(|_| ReportError::Malformed)?;
    let wire = [&[VERSION][..], &nonce, &sealed].concat();
    if wire.len() > MAX_SEALED_BYTES {
        return Err(ReportError::TooBig);
    }
    Ok(wire)
}

/// Opens a sealed report sent for `stick_id`: refuses anything too big,
/// sealed with another key or for another stick, changed, not signed by the
/// reader it names, or with a bad name. Never panics, whatever the bytes.
pub fn open(bytes: &[u8], stick_id: &StickId, stick_key: &StickKey) -> Result<Opened, ReportError> {
    if bytes.len() > MAX_SEALED_BYTES {
        return Err(ReportError::TooBig);
    }
    if bytes.len() < 1 + NONCE_LEN + TAG_LEN {
        return Err(ReportError::Malformed);
    }
    if bytes[0] != VERSION {
        return Err(ReportError::UnknownVersion);
    }
    let (nonce, sealed) = bytes[1..].split_at(NONCE_LEN);
    let plain = aead(stick_key)
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &associated(stick_id),
            },
        )
        .map_err(|_| ReportError::Sealed)?;
    if plain.len() < 32 + SIGNATURE_LEN {
        return Err(ReportError::Malformed);
    }
    let (id, rest) = plain.split_at(32);
    let (signature, body) = rest.split_at(SIGNATURE_LEN);
    let id: [u8; 32] = id.try_into().map_err(|_| ReportError::Malformed)?;
    let signature: [u8; SIGNATURE_LEN] =
        signature.try_into().map_err(|_| ReportError::Malformed)?;
    let verifying = VerifyingKey::from_bytes(&id).map_err(|_| ReportError::BadSignature)?;
    verifying
        .verify_strict(
            &signed_message(stick_id, body),
            &Signature::from_bytes(&signature),
        )
        .map_err(|_| ReportError::BadSignature)?;
    let (report, left): (ReaderReport, &[u8]) =
        postcard::take_from_bytes(body).map_err(|_| ReportError::Malformed)?;
    if !left.is_empty() {
        return Err(ReportError::Malformed);
    }
    check(&report)?;
    Ok(Opened {
        reader: ReaderId::from_bytes(id),
        report,
    })
}

/// What a report may say: a percent below 100, a name of the receiver's
/// within the rules.
fn check(report: &ReaderReport) -> Result<(), ReportError> {
    if let ReportState::Downloading { percent } = report.state {
        if percent > 99 {
            return Err(ReportError::Malformed);
        }
    }
    if let Some(name) = &report.name {
        if !name_is_allowed(name) {
            return Err(ReportError::BadName);
        }
    }
    Ok(())
}

/// A name a receiver may send: not empty, at most [`MAX_NAME_CHARS`], no
/// control, format or separator character (as a manifest's names).
pub fn name_is_allowed(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name
            .chars()
            .any(|c| c.is_control() || crate::manifest::is_format_or_separator(c))
}

fn aead(stick_key: &StickKey) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(&blake3::derive_key(KEY_CONTEXT, stick_key.as_bytes()).into())
}

fn associated(stick_id: &StickId) -> Vec<u8> {
    [&[VERSION][..], &stick_id.to_bytes()].concat()
}

fn signed_message(stick_id: &StickId, body: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, &[VERSION], &stick_id.to_bytes(), body].concat()
}

#[cfg(test)]
mod tests;
