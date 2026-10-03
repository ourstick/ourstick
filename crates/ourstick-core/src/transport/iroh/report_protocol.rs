//! `ourstick/report/1`: a reader tells the owner how far it is (ADR 0015).
//! See `docs/protocol.md`.
//!
//! One bidirectional stream. Request: the StickId (32 bytes), then the
//! sealed report; at most [`MAX_REPORT_BYTES`] in all, read with that limit.
//! Answer: nothing, the stream finished once the report is taken, so the
//! reader knows it arrived; reset when it is not taken (another stick, too
//! many waiting), so the reader keeps it to send again. The transport opens nothing:
//! it hands the sealed bytes to the owner's side, which holds the key.

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::Endpoint;
use tokio::sync::mpsc;

use crate::report::{MAX_REPORT_BYTES, MAX_SEALED_BYTES};
use crate::stick_id::StickId;
use crate::ticket::Ticket;
use crate::transport::TransportError;

use super::{bind, owner_addr, IrohConfig};

/// ALPN of the report protocol.
pub const REPORT_ALPN: &[u8] = b"ourstick/report/1";

/// How many reports may wait for the owner's side; more are refused, so a
/// flood costs the owner no memory and no work beyond reading them.
pub(super) const WAITING_REPORTS: usize = 32;

/// How long a peer may keep a connection open after its report: a peer
/// that never closes holds nothing longer (shared with the record server).
pub(super) const LINGER: std::time::Duration = std::time::Duration::from_secs(10);

/// What a stream is reset with when its report is not taken.
const NOT_TAKEN: u32 = 1;

/// The owner's side: takes reports for its stick and hands them on.
#[derive(Debug, Clone)]
pub(super) struct ReportServer {
    pub(super) stick_id: StickId,
    pub(super) reports: mpsc::Sender<Vec<u8>>,
}

impl ReportServer {
    /// The sealed report of a request for this stick, if it is one.
    fn sealed(&self, request: &[u8]) -> Option<Vec<u8>> {
        let (stick, sealed) = request.split_at_checked(32)?;
        (stick == self.stick_id.to_bytes() && !sealed.is_empty()).then(|| sealed.to_vec())
    }
}

impl ProtocolHandler for ReportServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        // Longer than a report can be: refused unread past the limit.
        let request = recv
            .read_to_end(MAX_REPORT_BYTES)
            .await
            .map_err(AcceptError::from_err)?;
        let taken = match self.sealed(&request) {
            Some(sealed) => {
                let taken = self.reports.try_send(sealed).is_ok();
                if !taken {
                    tracing::debug!("a reader's report not taken: too many waiting");
                }
                taken
            }
            None => {
                tracing::debug!("a request on the report protocol not for this stick");
                false
            }
        };
        if taken {
            send.finish()?;
        } else {
            send.reset(NOT_TAKEN.into()).ok();
        }
        linger(&connection).await;
        Ok(())
    }
}

/// Waits for the peer to close, at most [`LINGER`], then closes.
pub(super) async fn linger(connection: &Connection) {
    if tokio::time::timeout(LINGER, connection.closed())
        .await
        .is_err()
    {
        connection.close(0u32.into(), b"done");
    }
}

/// The reader's side: sends one sealed report and waits until the owner
/// has taken it; an error if it was not.
pub(super) async fn send_report(
    connection: &Connection,
    stick_id: &StickId,
    sealed: &[u8],
) -> Result<(), TransportError> {
    if sealed.len() > MAX_SEALED_BYTES {
        return Err(TransportError::Protocol("a report is too long".to_owned()));
    }
    let interrupted =
        |error: &dyn std::fmt::Display| TransportError::Interrupted(error.to_string());
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| interrupted(&error))?;
    send.write_all(&[&stick_id.to_bytes()[..], sealed].concat())
        .await
        .map_err(|error| interrupted(&error))?;
    send.finish()
        .map_err(|error| TransportError::Protocol(error.to_string()))?;
    recv.read_to_end(0)
        .await
        .map_err(|error| interrupted(&error))?;
    Ok(())
}

/// Sends reports and nothing else (ADR 0015): an endpoint with no blob
/// store, for telling a pause or a forget while the stick's own store is
/// in use or about to go.
#[derive(Debug)]
pub struct IrohReportSender {
    endpoint: Endpoint,
}

impl IrohReportSender {
    pub async fn start(config: &IrohConfig) -> Result<Self, TransportError> {
        Ok(Self {
            endpoint: bind(config, Vec::new()).await?,
        })
    }

    /// Closes its endpoint.
    pub async fn close(self) {
        self.endpoint.close().await;
    }
}

impl crate::download::ReportTo for IrohReportSender {
    async fn report(&self, ticket: &Ticket, sealed: &[u8]) -> Result<(), TransportError> {
        report_on(&self.endpoint, ticket, sealed).await
    }
}

/// One report on a connection of its own from `endpoint` (one QUIC
/// connection speaks one protocol).
pub(super) async fn report_on(
    endpoint: &Endpoint,
    ticket: &Ticket,
    sealed: &[u8],
) -> Result<(), TransportError> {
    let connection = endpoint
        .connect(owner_addr(ticket)?, REPORT_ALPN)
        .await
        .map_err(|error| TransportError::Unreachable(error.to_string()))?;
    let sent = send_report(&connection, &ticket.stick_id, sealed).await;
    connection.close(0u32.into(), b"done");
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (ReportServer, mpsc::Receiver<Vec<u8>>) {
        let (reports, taken) = mpsc::channel(WAITING_REPORTS);
        let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        (ReportServer { stick_id, reports }, taken)
    }

    #[test]
    fn only_a_request_for_this_stick_with_a_report_is_taken() {
        let (server, _taken) = server();
        let mine = server.stick_id.to_bytes();
        let other = crate::stick_id::StickSigningKey::from_bytes(&[2; 32])
            .stick_id()
            .to_bytes();
        assert_eq!(
            server.sealed(&[&mine[..], b"sealed"].concat()),
            Some(b"sealed".to_vec())
        );
        assert_eq!(server.sealed(&[&other[..], b"sealed"].concat()), None);
        assert_eq!(server.sealed(&mine), None);
        assert_eq!(server.sealed(&mine[..31]), None);
        assert_eq!(server.sealed(&[]), None);
    }
}
