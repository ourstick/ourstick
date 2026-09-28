//! Signed manifest records: the owner's statement "the current manifest of this
//! stick is `manifest_hash`, version `seq`".
//!
//! Fixed 137-byte layout, see `docs/protocol.md`. A reader only trusts a record
//! through [`SignedManifestRecord::verify`], which checks the stick, the
//! signature, and that the record is newer than anything already accepted.

use crate::stick_id::{StickId, StickSigningKey};

/// Version of the record format.
pub const RECORD_FORMAT_VERSION: u8 = 1;

/// Length in bytes of an encoded [`SignedManifestRecord`].
pub const SIGNED_RECORD_LEN: usize = 137;

/// Prefix of every signed message, so a record signature can never be taken
/// for a signature over anything else.
const SIGNATURE_CONTEXT: &[u8; 28] = b"OurStick manifest record v1\0";

/// Length of the part of a record that the signature covers (all but the signature).
const SIGNED_PART_LEN: usize = 73;

/// The content of a record, after it has been verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestRecord {
    pub stick_id: StickId,
    pub seq: u64,
    pub manifest_hash: [u8; 32],
}

impl ManifestRecord {
    fn signed_part(&self) -> [u8; SIGNED_PART_LEN] {
        let mut bytes = [0u8; SIGNED_PART_LEN];
        bytes[0] = RECORD_FORMAT_VERSION;
        bytes[1..33].copy_from_slice(&self.stick_id.to_bytes());
        bytes[33..41].copy_from_slice(&self.seq.to_be_bytes());
        bytes[41..73].copy_from_slice(&self.manifest_hash);
        bytes
    }

    fn message(&self) -> Vec<u8> {
        [&SIGNATURE_CONTEXT[..], &self.signed_part()].concat()
    }
}

/// The manifest a ticket was made with. Every reader has one, so it is the
/// oldest record a reader can ever accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestHint {
    pub seq: u64,
    pub hash: [u8; 32],
}

/// A record with its signature, as sent between peers. Its content is only
/// reachable through [`SignedManifestRecord::verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedManifestRecord {
    record: ManifestRecord,
    signature: [u8; 64],
}

/// Why a record was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("manifest record has {0} bytes, expected {SIGNED_RECORD_LEN}")]
    WrongLength(usize),
    #[error("unsupported manifest record version {0}")]
    UnsupportedVersion(u8),
    #[error("manifest record has an invalid stick id")]
    InvalidStickId,
    #[error("manifest record belongs to another stick")]
    WrongStick,
    #[error("manifest record signature is invalid")]
    BadSignature,
    #[error("manifest record {seq} is older than {ticket_seq}, the one in the ticket")]
    OlderThanTicket { seq: u64, ticket_seq: u64 },
    #[error("manifest record {seq} names a different manifest than the ticket with the same seq")]
    ConflictsWithTicket { seq: u64 },
    #[error("manifest record {seq} is not newer than {last_seen}, the latest already accepted")]
    NotNewer { seq: u64, last_seen: u64 },
}

/// The `seq` for the owner's next record: `max(last_signed_seq + 1, now_ms)`.
///
/// The clock term keeps `seq` ahead even if the stored value was lost or rolled
/// back; the `+ 1` term keeps it increasing when the clock stalls or goes back.
/// Returns `None` only if `last_signed_seq` is already `u64::MAX`.
/// See `docs/decisions/0004-signed-manifest-records.md`.
pub fn next_seq(last_signed_seq: u64, now_ms: u64) -> Option<u64> {
    Some(last_signed_seq.checked_add(1)?.max(now_ms))
}

impl StickSigningKey {
    /// Signs the record `(this stick, seq, manifest_hash)`. The caller must use a
    /// `seq` greater than any it has signed before for this stick.
    pub fn sign_record(&self, seq: u64, manifest_hash: [u8; 32]) -> SignedManifestRecord {
        let record = ManifestRecord {
            stick_id: self.stick_id(),
            seq,
            manifest_hash,
        };
        SignedManifestRecord {
            signature: self.sign(&record.message()),
            record,
        }
    }
}

impl SignedManifestRecord {
    /// The record's content without checking the signature. Only for the owner's
    /// own records (e.g. to store them); readers must use [`Self::verify`].
    pub(crate) fn unverified_record(&self) -> &ManifestRecord {
        &self.record
    }

    pub fn to_bytes(&self) -> [u8; SIGNED_RECORD_LEN] {
        let mut bytes = [0u8; SIGNED_RECORD_LEN];
        bytes[..SIGNED_PART_LEN].copy_from_slice(&self.record.signed_part());
        bytes[SIGNED_PART_LEN..].copy_from_slice(&self.signature);
        bytes
    }

    /// Parses a record. This checks the layout only, not the signature.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RecordError> {
        let bytes: &[u8; SIGNED_RECORD_LEN] = bytes
            .try_into()
            .map_err(|_| RecordError::WrongLength(bytes.len()))?;
        if bytes[0] != RECORD_FORMAT_VERSION {
            return Err(RecordError::UnsupportedVersion(bytes[0]));
        }
        let stick_id = StickId::from_bytes(bytes[1..33].try_into().expect("32 bytes"))
            .map_err(|_| RecordError::InvalidStickId)?;
        let record = ManifestRecord {
            stick_id,
            seq: u64::from_be_bytes(bytes[33..41].try_into().expect("8 bytes")),
            manifest_hash: bytes[41..73].try_into().expect("32 bytes"),
        };
        Ok(Self {
            record,
            signature: bytes[SIGNED_PART_LEN..].try_into().expect("64 bytes"),
        })
    }

    /// Accepts the record only if, in this order:
    ///
    /// 1. it belongs to `expected` (the StickId from the ticket),
    /// 2. its signature verifies,
    /// 3. it is not older than `ticket_hint`, and if it has the same `seq`, it
    ///    names the same manifest,
    /// 4. its `seq` is strictly greater than `last_seen`, the highest `seq`
    ///    already accepted for this stick (if any).
    pub fn verify(
        &self,
        expected: &StickId,
        ticket_hint: &ManifestHint,
        last_seen: Option<u64>,
    ) -> Result<ManifestRecord, RecordError> {
        if self.record.stick_id != *expected {
            return Err(RecordError::WrongStick);
        }
        if !expected.verify_strict(&self.record.message(), &self.signature) {
            return Err(RecordError::BadSignature);
        }
        let seq = self.record.seq;
        if seq < ticket_hint.seq {
            return Err(RecordError::OlderThanTicket {
                seq,
                ticket_seq: ticket_hint.seq,
            });
        }
        if seq == ticket_hint.seq && self.record.manifest_hash != ticket_hint.hash {
            return Err(RecordError::ConflictsWithTicket { seq });
        }
        if let Some(last_seen) = last_seen {
            if self.record.seq <= last_seen {
                return Err(RecordError::NotNewer {
                    seq: self.record.seq,
                    last_seen,
                });
            }
        }
        Ok(self.record)
    }
}

#[cfg(test)]
mod tests;
