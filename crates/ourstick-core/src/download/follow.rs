//! `get --follow`: after a download, wait for the owner's next version by
//! asking for its signed record now and then (137 bytes each time).

use std::time::Duration;

use super::DownloadError;
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PeerConnection, TransportError};

/// How often `get --follow` asks for the owner's record.
pub const FOLLOW_EVERY: Duration = Duration::from_secs(15);

/// What happens while waiting, for display.
#[derive(Debug)]
pub enum FollowEvent<'a> {
    /// The owner cannot be reached; still trying. Reported once per outage.
    Unreachable { reason: &'a TransportError },
    /// The owner can be reached again.
    Reachable,
}

/// Waits until the owner offers a version newer than `seq`, asking every
/// `interval` over one connection (reconnecting when it drops), and returns
/// its `seq`. The record is verified as always; a bad one is an error. The
/// download that follows checks it against the floor again.
pub async fn wait_for_newer(
    fetcher: &impl Fetcher,
    ticket: &Ticket,
    seq: u64,
    interval: Duration,
    on_event: &mut (dyn FnMut(FollowEvent<'_>) + Send),
) -> Result<u64, DownloadError> {
    let mut connection = None;
    let mut unreachable = false;
    loop {
        let asked = match connection.take() {
            Some(connection) => Ok(connection),
            None => fetcher.connect(ticket).await,
        };
        let answer = match asked {
            Ok(open) => {
                let answer = open.fetch_record(&ticket.stick_id).await;
                if answer.is_ok() {
                    connection = Some(open);
                } else {
                    open.close().await;
                }
                answer
            }
            Err(error) => Err(error),
        };
        match answer {
            Ok(signed) => {
                let record = signed.verify(&ticket.stick_id, &ticket.manifest_hint, None)?;
                if unreachable {
                    unreachable = false;
                    on_event(FollowEvent::Reachable);
                }
                if record.seq > seq {
                    if let Some(open) = connection.take() {
                        open.close().await;
                    }
                    return Ok(record.seq);
                }
            }
            Err(reason) => {
                if !unreachable {
                    unreachable = true;
                    on_event(FollowEvent::Unreachable { reason: &reason });
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}
