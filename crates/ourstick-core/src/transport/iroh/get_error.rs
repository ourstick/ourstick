//! A failed fetch from the owner, in our own terms.

use iroh_blobs::get::fsm::DecodeError;
use iroh_blobs::get::GetError;
use iroh_blobs::protocol::{ERR_INTERNAL, ERR_LIMIT, ERR_PERMISSION};

use crate::transport::TransportError;

fn is_hash_mismatch(error: &GetError) -> bool {
    matches!(
        error,
        GetError::Decode {
            source: DecodeError::LeafHashMismatch { .. } | DecodeError::ParentHashMismatch { .. },
            ..
        }
    )
}

pub(super) fn get_error(error: GetError) -> TransportError {
    if is_hash_mismatch(&error) {
        TransportError::Corrupt
    } else if error.iroh_error_code() == Some(ERR_PERMISSION) {
        TransportError::Refused
    } else if error.iroh_error_code() == Some(ERR_LIMIT) {
        // "OK to try again later" (iroh-blobs): the owner is preparing the
        // blob (ADR 0007).
        TransportError::NotReady
    } else if error.iroh_error_code() == Some(ERR_INTERNAL) {
        // The owner checks each part against the blob's tree before it sends
        // it, and stops with this code when its file no longer gives that
        // blob (it changed, ADR 0017).
        TransportError::OwnerFailed
    } else {
        TransportError::Interrupted(error.to_string())
    }
}
