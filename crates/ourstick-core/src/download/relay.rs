//! When file data may go through a relay (ADR 0006): a short wait for a direct
//! path, then up to a budget through the relay while iroh keeps looking for a
//! direct path; once the budget is used up, only a direct path, unless the
//! user agrees.

use std::future::Future;
use std::time::Duration;

use crate::ticket::Ticket;
use crate::transport::{relay_reasons, Fetcher, PeerConnection};

use super::{DownloadError, DownloadEvent};

/// The numbers of the relay policy. The defaults are those of ADR 0006.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPolicy {
    /// File data that may arrive through the relay per download without asking.
    pub relay_budget: u64,
    /// How long to wait for a direct path before starting at all.
    pub first_wait: Duration,
    /// How long to wait for a direct path once the budget is used up.
    pub budget_wait: Duration,
}

impl Default for RelayPolicy {
    fn default() -> Self {
        Self {
            relay_budget: 100 * 1000 * 1000,
            first_wait: Duration::from_secs(5),
            budget_wait: Duration::from_secs(15),
        }
    }
}

/// What the user is asked once the relay budget is used up and no direct path
/// opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayQuestion {
    /// Bytes still to download.
    pub remaining: u64,
    /// Bytes that already came through the relay in this download.
    pub relay_used: u64,
    /// Which relay would carry the data, as the transport describes it.
    pub relay: String,
    /// Likely reasons there is no direct path, most likely first.
    pub reasons: Vec<String>,
    /// True if a direct path was found after the budget was used up, then lost.
    pub direct_was_lost: bool,
}

/// Decides whether a transfer may continue through the relay.
pub trait RelayConsent: Send + Sync {
    /// `true` to continue through the relay, `false` to stop (resumable).
    fn allow_relay(&self, question: RelayQuestion) -> impl Future<Output = bool> + Send;
}

/// What the relay policy has settled for this download so far, across
/// reconnects.
#[derive(Debug, Default)]
pub(super) struct RelayState {
    /// The user agreed to the relay; no more waiting or asking.
    pub(super) consented: bool,
    /// The budget was used up; from now on only a direct path is acceptable.
    pub(super) budget_spent: bool,
    /// Relay bytes of earlier connections of this download.
    pub(super) relay_used: u64,
    /// A direct path was found after the budget was used up, then lost.
    pub(super) direct_was_lost: bool,
}

/// What to watch for while file data arrives over one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Watch {
    /// The user agreed to the relay: nothing.
    Nothing,
    /// Stop once relay bytes of this download exceed `budget`. `earlier` came
    /// through the relay on earlier connections, `baseline` is this
    /// connection's relay count when its file data started.
    Budget {
        budget: u64,
        earlier: u64,
        baseline: u64,
    },
    /// Only a direct path is acceptable: stop as soon as data arrives through
    /// the relay.
    DirectOnly,
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

/// Decides, before file data is fetched over this connection, what to watch
/// for; waits and asks as ADR 0006 says.
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
) -> Result<Watch, DownloadError> {
    if state.consented {
        return Ok(Watch::Nothing);
    }
    let wait = if state.budget_spent {
        policy.budget_wait
    } else {
        policy.first_wait
    };
    let direct = connection.has_direct_path() || {
        if on_event(DownloadEvent::WaitingForDirect {
            seconds: wait.as_secs(),
            budget_used: state.budget_spent,
        })
        .is_break()
        {
            return Err(DownloadError::Cancelled);
        }
        wait_for_direct(connection, wait).await
    };
    if !state.budget_spent {
        // Direct or not, start: the budget covers the relay meanwhile.
        return Ok(Watch::Budget {
            budget: policy.relay_budget,
            earlier: state.relay_used,
            baseline: connection.usage().relay,
        });
    }
    if direct {
        return Ok(Watch::DirectOnly);
    }
    let question = RelayQuestion {
        remaining,
        relay_used: state.relay_used,
        relay: fetcher.relay_description(),
        reasons: relay_reasons(fetcher.network_report().as_ref(), &ticket.address_hints),
        direct_was_lost: state.direct_was_lost,
    };
    if consent.allow_relay(question).await {
        state.consented = true;
        Ok(Watch::Nothing)
    } else {
        Err(DownloadError::RelayDeclined { remaining })
    }
}

impl Watch {
    /// Whether to stop after a check that found the data arriving over
    /// `arriving` with `relay_now` relay bytes on this connection.
    pub(super) fn must_stop(&self, arriving_by_relay: bool, relay_now: u64) -> Option<Stop> {
        match *self {
            Watch::Nothing => None,
            Watch::Budget {
                budget,
                earlier,
                baseline,
            } => {
                let used = earlier + relay_now.saturating_sub(baseline);
                (used > budget).then_some(Stop::BudgetUsed)
            }
            Watch::DirectOnly => arriving_by_relay.then_some(Stop::DirectLost),
        }
    }
}

/// Why a fetch was stopped by the relay policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    BudgetUsed,
    DirectLost,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_adr_0006() {
        let policy = RelayPolicy::default();
        assert_eq!(policy.relay_budget, 100_000_000);
        assert_eq!(policy.first_wait, Duration::from_secs(5));
        assert_eq!(policy.budget_wait, Duration::from_secs(15));
    }

    #[test]
    fn budget_counts_earlier_connections_and_this_one_since_its_start() {
        let watch = Watch::Budget {
            budget: 100,
            earlier: 60,
            baseline: 10,
        };
        assert_eq!(watch.must_stop(true, 50), None); // 60 + 40 = 100
        assert_eq!(watch.must_stop(true, 51), Some(Stop::BudgetUsed));
        assert_eq!(watch.must_stop(false, 51), Some(Stop::BudgetUsed));
    }

    #[test]
    fn direct_only_stops_on_relay_data() {
        assert_eq!(Watch::DirectOnly.must_stop(false, 1_000), None);
        assert_eq!(Watch::DirectOnly.must_stop(true, 0), Some(Stop::DirectLost));
        assert_eq!(Watch::Nothing.must_stop(true, u64::MAX), None);
    }
}
