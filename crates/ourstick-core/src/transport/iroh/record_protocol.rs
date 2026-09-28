//! `ourstick/record/1`: a reader asks for the latest signed manifest record of a
//! stick. See `docs/protocol.md`.
//!
//! Request: the StickId (32 bytes). Response: one status byte, then for status
//! 0 the signed record (137 bytes).

use std::sync::{Arc, RwLock};

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};

use crate::record::{SignedManifestRecord, SIGNED_RECORD_LEN};
use crate::stick_id::StickId;
use crate::transport::TransportError;

/// ALPN of the record protocol.
pub const RECORD_ALPN: &[u8] = b"ourstick/record/1";

const STATUS_OK: u8 = 0;
const STATUS_UNKNOWN_STICK: u8 = 1;
const STATUS_NO_RECORD: u8 = 2;

/// The owner's side: answers with the current record of one stick.
#[derive(Debug, Clone)]
pub(super) struct RecordServer {
    pub(super) stick_id: StickId,
    pub(super) record: Arc<RwLock<Option<SignedManifestRecord>>>,
}

impl RecordServer {
    fn answer(&self, request: &[u8]) -> Vec<u8> {
        if request != self.stick_id.to_bytes() {
            return vec![STATUS_UNKNOWN_STICK];
        }
        let record = self
            .record
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match record {
            Some(record) => [&[STATUS_OK][..], &record.to_bytes()].concat(),
            None => vec![STATUS_NO_RECORD],
        }
    }
}

impl ProtocolHandler for RecordServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv.read_to_end(32).await.map_err(AcceptError::from_err)?;
        send.write_all(&self.answer(&request))
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

/// The reader's side: one request on a fresh connection.
pub(super) async fn request_record(
    connection: &Connection,
    stick_id: &StickId,
) -> Result<SignedManifestRecord, TransportError> {
    let protocol = |error: &dyn std::fmt::Display| TransportError::Protocol(error.to_string());
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| TransportError::Interrupted(error.to_string()))?;
    send.write_all(&stick_id.to_bytes())
        .await
        .map_err(|error| TransportError::Interrupted(error.to_string()))?;
    send.finish().map_err(|error| protocol(&error))?;
    let response = recv
        .read_to_end(1 + SIGNED_RECORD_LEN)
        .await
        .map_err(|error| protocol(&error))?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<SignedManifestRecord, TransportError> {
    match response.split_first() {
        Some((&STATUS_OK, record)) => SignedManifestRecord::from_bytes(record)
            .map_err(|error| TransportError::Protocol(error.to_string())),
        Some((&STATUS_UNKNOWN_STICK, [])) => Err(TransportError::UnknownStick),
        Some((&STATUS_NO_RECORD, [])) => Err(TransportError::NoRecord),
        _ => Err(TransportError::Protocol(
            "unexpected record response".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stick_id::StickSigningKey;

    fn server(record: Option<SignedManifestRecord>) -> RecordServer {
        RecordServer {
            stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
            record: Arc::new(RwLock::new(record)),
        }
    }

    #[test]
    fn answers_roundtrip_through_parse() {
        let owner = StickSigningKey::from_bytes(&[1; 32]);
        let record = owner.sign_record(7, [9; 32]);
        let request = owner.stick_id().to_bytes();

        let answer = server(Some(record.clone())).answer(&request);
        assert_eq!(answer.len(), 1 + SIGNED_RECORD_LEN);
        assert_eq!(parse_response(&answer).unwrap(), record);

        assert!(matches!(
            parse_response(&server(None).answer(&request)),
            Err(TransportError::NoRecord)
        ));
        assert!(matches!(
            parse_response(&server(Some(record)).answer(&[0; 32])),
            Err(TransportError::UnknownStick)
        ));
    }

    #[test]
    fn parse_rejects_malformed_responses() {
        for response in [&[][..], &[0], &[0; 10], &[1, 0], &[3]] {
            assert!(matches!(
                parse_response(response),
                Err(TransportError::Protocol(_))
            ));
        }
    }
}
