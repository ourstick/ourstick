//! `get --follow`: after a download, wait for the owner's next version by
//! asking for its signed record now and then (137 bytes each time).

use std::time::Duration;

use std::ops::ControlFlow;

use super::{DownloadError, DownloadEvent};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PeerConnection, TransportError};

/// How often `get --follow` asks for the owner's record, at first.
pub const FOLLOW_EVERY: Duration = Duration::from_secs(15);
/// After this long with no new version, the owner is asked less and less
/// often, up to [`FOLLOW_LONGEST`] apart: a stick that stays the same for
/// days, or an owner that stays off, is not asked four times a minute.
const FOLLOW_STEADY: Duration = Duration::from_secs(10 * 60);
const FOLLOW_LONGEST: Duration = Duration::from_secs(2 * 60);

/// The wait before the next ask, after `asked` asks with no new version:
/// `interval` for [`FOLLOW_STEADY`], then twice as long each time, up to
/// [`FOLLOW_LONGEST`] (or `interval`, if longer).
fn wait_after(interval: Duration, asked: u32) -> Duration {
    let steady =
        u32::try_from(FOLLOW_STEADY.as_millis() / interval.as_millis().max(1)).unwrap_or(u32::MAX);
    let over = asked.saturating_sub(steady);
    interval
        .saturating_mul(1 << over.min(16))
        .min(FOLLOW_LONGEST.max(interval))
}

/// What happens while waiting, for display.
#[derive(Debug)]
pub enum FollowEvent<'a> {
    /// The owner cannot be reached; still trying. Reported once per outage.
    Unreachable { reason: &'a TransportError },
    /// The owner can be reached again.
    Reachable,
}

/// Waits until the owner offers a version newer than `seq`, asking every
/// `interval` (less often after a while, see [`wait_after`]), and returns
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
    let mut asks_in_vain = 0;
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
        tokio::time::sleep(wait_after(interval, asks_in_vain)).await;
        asks_in_vain = asks_in_vain.saturating_add(1);
    }
}

#[cfg(test)]
#[test]
fn the_owner_is_asked_less_often_after_a_while() {
    let every = FOLLOW_EVERY;
    // Ten minutes at 15 s: 40 asks.
    assert_eq!(wait_after(every, 0), every);
    assert_eq!(wait_after(every, 40), every);
    assert_eq!(wait_after(every, 41), every * 2);
    assert_eq!(wait_after(every, 42), every * 4);
    assert_eq!(wait_after(every, 43), FOLLOW_LONGEST);
    assert_eq!(wait_after(every, u32::MAX), FOLLOW_LONGEST);
    // A short interval (tests) and a long one are kept within bounds.
    assert_eq!(
        wait_after(Duration::from_millis(50), 0),
        Duration::from_millis(50)
    );
    let long = Duration::from_secs(600);
    assert_eq!(wait_after(long, 5), long);
}

/// Reaching the owner during a download. Once it said more is coming (a
/// partial version), an owner that cannot be reached is waited for, as
/// while waiting for its next version, not the end of the download.
#[derive(Debug, Default)]
pub(super) struct Reach {
    pub(super) more_coming: bool,
    unreachable: bool,
}

impl Reach {
    /// A connection to the owner; `None` after waiting [`FOLLOW_EVERY`] for
    /// an owner that more is coming from but cannot be reached now (told
    /// once, and again when it is back).
    pub(super) async fn connect<F: Fetcher>(
        &mut self,
        fetcher: &F,
        ticket: &Ticket,
        on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> ControlFlow<()> + Send),
    ) -> Result<Option<F::Connection>, DownloadError> {
        match fetcher.connect(ticket).await {
            Ok(connection) => {
                if self.unreachable {
                    self.unreachable = false;
                    if on_event(DownloadEvent::OwnerReachable).is_break() {
                        return Err(DownloadError::Cancelled);
                    }
                }
                Ok(Some(connection))
            }
            Err(reason @ TransportError::Unreachable(_)) if self.more_coming => {
                if !self.unreachable {
                    self.unreachable = true;
                    if on_event(DownloadEvent::OwnerUnreachable { reason: &reason }).is_break() {
                        return Err(DownloadError::Cancelled);
                    }
                }
                tokio::time::sleep(FOLLOW_EVERY).await;
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}
