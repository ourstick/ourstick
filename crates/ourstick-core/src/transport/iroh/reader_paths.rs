//! Which paths readers reach the owner on, seen from the owner. An owner
//! whose firewall drops what others send it (e.g. after "Cancel" on the
//! Windows Firewall question) is never reached directly: every reader comes
//! through the relay, slower and limited. The app tells the user so, with
//! what to do; this notices it.

use std::time::Duration;

use futures::StreamExt;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_blobs::BlobsProtocol;

use super::activity::Activity;
use super::is_relay;

/// How long a reader may stay on the relay before it counts as reaching the
/// owner only through it. Finding a direct path takes a few seconds.
pub(super) const RELAY_ONLY_AFTER: Duration = Duration::from_secs(30);

/// The blobs protocol, with an eye on each reader's paths.
#[derive(Debug, Clone)]
pub(super) struct WatchedBlobs {
    pub(super) blobs: BlobsProtocol,
    pub(super) activity: Activity,
    pub(super) relay_only_after: Duration,
}

impl ProtocolHandler for WatchedBlobs {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let watch = tokio::spawn(watch_paths(
            connection.clone(),
            self.activity.clone(),
            self.relay_only_after,
        ));
        let result = self.blobs.accept(connection).await;
        watch.abort();
        result
    }

    async fn shutdown(&self) {
        self.blobs.shutdown().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selected {
    Direct,
    Relay,
    Nothing,
}

fn selected(connection: &Connection) -> Selected {
    let paths = connection.paths();
    match paths.iter().find(|path| path.is_selected()) {
        Some(path) if is_relay(path.remote_addr()) => Selected::Relay,
        Some(_) => Selected::Direct,
        None => Selected::Nothing,
    }
}

/// Tells `activity` once the reader on `connection` is reached directly, or
/// that it is still on the relay after `relay_only_after`.
async fn watch_paths(connection: Connection, activity: Activity, relay_only_after: Duration) {
    let mut events = std::pin::pin!(connection.path_events());
    let deadline = tokio::time::sleep(relay_only_after);
    let mut deadline = std::pin::pin!(deadline);
    let mut counted = false;
    loop {
        if selected(&connection) == Selected::Direct {
            activity.reached_directly();
            return;
        }
        tokio::select! {
            event = events.next() => {
                if event.is_none() {
                    return;
                }
            }
            () = &mut deadline, if !counted => {
                counted = true;
                if selected(&connection) == Selected::Relay {
                    activity.reached_only_by_relay();
                }
            }
        }
    }
}
