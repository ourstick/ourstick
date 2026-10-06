//! What a download carries from one connection to the next.

use super::{relay_watch, DownloadError};

/// What carries over from one connection of a download to the next.
#[derive(Debug, Default)]
pub(super) struct Carried {
    /// Blob bytes received, over all connections (a stopped fetch counts too).
    pub(super) received: u64,
    /// The version the last connection was downloading, once known.
    pub(super) seq: Option<u64>,
    /// The owner refused data of this version (`None`: before it was known):
    /// the next connection continues only if the owner has a newer one.
    pub(super) refused: Option<(Option<u64>, DownloadError)>,
    /// How long the download has been on the relay, over connections.
    pub(super) watch: relay_watch::RelayWatch,
    /// Connections lost in a row with nothing received in between.
    pub(super) lost: u32,
    /// `received` when the last connection was lost.
    pub(super) received_at_loss: u64,
}

/// Connections lost in a row, with nothing received in between, before the
/// download stops: an owner that answers and then drops every connection.
pub(super) const MAX_LOST_IN_A_ROW: u32 = 5;
