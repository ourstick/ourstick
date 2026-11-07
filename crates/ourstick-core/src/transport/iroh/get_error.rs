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

/// This computer's own store failed, not the connection: since iroh-blobs
/// 0.104 a write into it that fails ends the fetch (`LocalFailure`, which no
/// longer covers opening the stream), and its import channel closing is
/// local too (`IrpcSend`).
fn is_local(error: &GetError) -> bool {
    matches!(
        error,
        GetError::LocalFailure { .. } | GetError::IrpcSend { .. }
    )
}

pub(super) fn get_error(error: GetError) -> TransportError {
    if is_hash_mismatch(&error) {
        TransportError::Corrupt
    } else if is_local(&error) {
        // Said with its causes ("local failure: ..."), and never as an owner
        // that cannot be reached.
        TransportError::Storage(format!("{error:#}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    // `LocalFailure` cannot be made outside iroh-blobs (it takes an
    // `n0_error::AnyError`); `is_local` puts it in the same arm.
    #[tokio::test]
    async fn a_failure_of_this_computers_store_is_not_a_lost_connection() {
        // What a fetch gets when the store's import channel has closed.
        let (sender, receiver) = irpc::channel::mpsc::channel::<u8>(1);
        drop(receiver);
        let error = GetError::from(sender.send(1).await.unwrap_err());
        assert!(is_local(&error));
        assert!(matches!(get_error(error), TransportError::Storage(_)));
    }
}
