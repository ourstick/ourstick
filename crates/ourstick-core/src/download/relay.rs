//! When file data may go through a relay (ADR 0006): small transfers always;
//! larger ones only over a direct path, unless the user agrees.

use std::future::Future;
use std::time::Duration;

use crate::ticket::Ticket;
use crate::transport::{relay_reasons, Fetcher, PeerConnection};

use super::{DownloadError, DownloadEvent};

/// The numbers of the relay policy. The defaults are those of ADR 0006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPolicy {
    /// Up to this many bytes left to download may go through the relay without
    /// asking.
    pub free_relay_bytes: u64,
    /// How long to wait for a direct path before asking.
    pub direct_wait: Duration,
}

impl Default for RelayPolicy {
    fn default() -> Self {
        Self {
            free_relay_bytes: 100 * 1000 * 1000,
            direct_wait: Duration::from_secs(15),
        }
    }
}

/// What the user is asked when a larger transfer has no direct path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayQuestion {
    /// Bytes still to download.
    pub remaining: u64,
    /// Which relay would carry the data, as the transport describes it.
    pub relay: String,
    /// Likely reasons there is no direct path, most likely first.
    pub reasons: Vec<String>,
    /// True if a direct path existed earlier in this download and was lost.
    pub direct_was_lost: bool,
}

/// Decides whether a larger transfer may continue through the relay.
pub trait RelayConsent: Send + Sync {
    /// `true` to continue through the relay, `false` to stop (resumable).
    fn allow_relay(&self, question: RelayQuestion) -> impl Future<Output = bool> + Send;
}

/// What the relay policy has settled for this download so far.
#[derive(Debug, Default)]
pub(super) struct RelayState {
    /// The user agreed to the relay; no more waiting or asking.
    pub(super) consented: bool,
    /// A direct path was lost during this download.
    pub(super) direct_was_lost: bool,
}

/// Waits until the connection has a direct path, for at most `wait`.
async fn wait_for_direct(connection: &impl PeerConnection, wait: Duration) -> bool {
    const POLL: Duration = Duration::from_millis(100);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if connection.has_direct_path() {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        tokio::time::sleep(POLL.min(deadline - now)).await;
    }
}

/// Decides, before file data is fetched, whether this attempt may use the
/// relay. Returns whether the data must be watched for arriving through the
/// relay (true when only a direct path is acceptable).
#[allow(clippy::too_many_arguments)]
pub(super) async fn settle(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    ticket: &Ticket,
    policy: &RelayPolicy,
    consent: &impl RelayConsent,
    state: &mut RelayState,
    remaining: u64,
    on_event: &mut (dyn FnMut(DownloadEvent<'_>) -> std::ops::ControlFlow<()> + Send),
) -> Result<bool, DownloadError> {
    if state.consented || remaining <= policy.free_relay_bytes {
        return Ok(false);
    }
    if on_event(DownloadEvent::WaitingForDirect {
        seconds: policy.direct_wait.as_secs(),
    })
    .is_break()
    {
        return Err(DownloadError::Cancelled);
    }
    if wait_for_direct(connection, policy.direct_wait).await {
        return Ok(true);
    }
    let question = RelayQuestion {
        remaining,
        relay: fetcher.relay_description(),
        reasons: relay_reasons(fetcher.network_report().as_ref(), &ticket.address_hints),
        direct_was_lost: state.direct_was_lost,
    };
    if consent.allow_relay(question).await {
        state.consented = true;
        Ok(false)
    } else {
        Err(DownloadError::RelayDeclined { remaining })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_adr_0006() {
        let policy = RelayPolicy::default();
        assert_eq!(policy.free_relay_bytes, 100_000_000);
        assert_eq!(policy.direct_wait, Duration::from_secs(15));
    }
}
