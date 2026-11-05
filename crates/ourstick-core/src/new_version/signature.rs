//! A minisign signature of the legacy kind (`Ed`: the bytes signed as they
//! are), checked before anything in a notice is parsed (ADR 0033 rule 3).
//! Any other kind is refused, so a notice signed the wrong way is seen at
//! the release, not ignored by every app.
//!
//! The form, four lines:
//! `untrusted comment: …`, base64 of `Ed` ‖ key id (8) ‖ signature (64),
//! `trusted comment: …`, base64 of the signature of (signature ‖ comment).

use data_encoding::BASE64;
use ed25519_dalek::Signature;

use super::keys::NoticeKey;

/// Which of the keys signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signer {
    Release,
    Recovery,
}

/// Why a signature is refused. Never holds the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("not a minisign signature")]
    NotASignature,
    #[error("not a legacy (Ed) minisign signature")]
    NotLegacy,
    #[error("signed by another key")]
    OtherKey,
    #[error("the signature does not match")]
    Mismatch,
}

/// Checks `signature` (a `.minisig` file's text) over `signed` with whichever
/// of `release` and `recovery` it names, and says which one.
pub fn check(
    signed: &[u8],
    signature: &str,
    release: &NoticeKey,
    recovery: &NoticeKey,
) -> Result<Signer, SignatureError> {
    let mut lines = signature.lines().map(|line| line.trim_end_matches('\r'));
    let (Some(untrusted), Some(main), Some(trusted), Some(global)) =
        (lines.next(), lines.next(), lines.next(), lines.next())
    else {
        return Err(SignatureError::NotASignature);
    };
    if !untrusted.starts_with("untrusted comment: ") || lines.any(|line| !line.trim().is_empty()) {
        return Err(SignatureError::NotASignature);
    }
    let comment = trusted
        .strip_prefix("trusted comment: ")
        .ok_or(SignatureError::NotASignature)?;
    let main = BASE64
        .decode(main.as_bytes())
        .map_err(|_| SignatureError::NotASignature)?;
    let global = BASE64
        .decode(global.as_bytes())
        .map_err(|_| SignatureError::NotASignature)?;
    if main.len() != 74 || global.len() != 64 {
        return Err(SignatureError::NotASignature);
    }
    if &main[..2] != b"Ed" {
        return Err(SignatureError::NotLegacy);
    }
    let id: [u8; 8] = main[2..10].try_into().expect("8 bytes");
    let (key, signer) = if id == release.id() {
        (release, Signer::Release)
    } else if id == recovery.id() {
        (recovery, Signer::Recovery)
    } else {
        return Err(SignatureError::OtherKey);
    };
    let main_signature = Signature::from_bytes(&main[10..74].try_into().expect("64 bytes"));
    key.key()
        .verify_strict(signed, &main_signature)
        .map_err(|_| SignatureError::Mismatch)?;
    // The trusted comment is signed too, with the signature before it.
    let mut both = main[10..74].to_vec();
    both.extend_from_slice(comment.as_bytes());
    let global = Signature::from_bytes(&global.try_into().expect("64 bytes"));
    key.key()
        .verify_strict(&both, &global)
        .map_err(|_| SignatureError::Mismatch)?;
    Ok(signer)
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;
