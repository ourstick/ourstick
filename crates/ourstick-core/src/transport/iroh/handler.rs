//! Our blobs protocol handler (ADR 0007): it reads each request itself, lets
//! [`Serving`] decide under one lock, and then serves the blob with
//! iroh-blobs' own `handle_get` from the store that holds it, or resets the
//! stream with the reason. iroh-blobs' events only tell how many bytes left,
//! for the reader activity; they decide nothing.

use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::Endpoint;
use iroh_blobs::protocol::{Request, ERR_LIMIT, ERR_PERMISSION};
use iroh_blobs::provider::events::{
    ClientConnected, ConnectionClosed, EventMask, EventSender, ObserveMode, ProviderMessage,
    RequestMode, RequestUpdate,
};
use iroh_blobs::provider::{handle_get, StreamPair};

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;

use super::activity::Activity;
use super::limits;
use super::local_network::watch_declined;
use super::reader_paths::watch_paths;
use super::serving::{Decision, Serving};
use crate::local_network::SharedProbe;
use crate::transport::Demand;

/// The blobs protocol of an owner.
#[derive(Debug, Clone)]
pub(super) struct BlobsHandler {
    pub(super) serving: Serving,
    pub(super) activity: Activity,
    pub(super) events: EventSender,
    pub(super) relay_only_after: Duration,
    /// What readers ask for, for the owner's policy (ADR 0007).
    pub(super) demand: UnboundedSender<Demand>,
    /// For each reader's addresses, and whether the local network is
    /// declined (ADR 0020).
    pub(super) endpoint: Endpoint,
    pub(super) probe: Arc<SharedProbe>,
    /// Places for connections at once (ADR 0031).
    connections: Arc<Semaphore>,
}

/// Only gets are asked for events, to count the bytes sent; every other
/// kind is refused by the handler before iroh-blobs sees it.
fn event_mask() -> EventMask {
    EventMask {
        get: RequestMode::NotifyLog,
        get_many: RequestMode::Disabled,
        push: RequestMode::Disabled,
        observe: ObserveMode::None,
        ..EventMask::DEFAULT
    }
}

impl BlobsHandler {
    /// The handler, and the task that counts the bytes sent into `activity`.
    pub(super) fn new(
        serving: Serving,
        activity: Activity,
        relay_only_after: Duration,
        demand: UnboundedSender<Demand>,
        endpoint: Endpoint,
        probe: Arc<SharedProbe>,
    ) -> Self {
        let (events, mut messages) = EventSender::channel(32, event_mask());
        let counting = activity.clone();
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                if let ProviderMessage::GetRequestReceivedNotify(message) = message {
                    let mut updates = message.rx;
                    let activity = counting.clone();
                    tokio::spawn(async move {
                        let mut offset = 0;
                        while let Ok(Some(update)) = updates.recv().await {
                            match update {
                                RequestUpdate::Started(_) => offset = 0,
                                RequestUpdate::Progress(progress) => {
                                    activity.sent(progress.end_offset.saturating_sub(offset));
                                    offset = progress.end_offset;
                                }
                                RequestUpdate::Completed(_) | RequestUpdate::Aborted(_) => {}
                            }
                        }
                    });
                }
            }
        });
        Self {
            serving,
            activity,
            events,
            relay_only_after,
            endpoint,
            probe,
            demand,
            connections: Arc::new(Semaphore::new(limits::CONNECTIONS_PER_STICK)),
        }
    }
}

impl ProtocolHandler for BlobsHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        // Node ids cost nothing: a stick takes so many connections at once.
        let Ok(_place) = self.connections.clone().try_acquire_owned() else {
            tracing::debug!("a connection refused: the stick has all it takes");
            connection.close(0u32.into(), b"busy");
            return Ok(());
        };
        let id = connection.stable_id() as u64;
        let watch = tokio::spawn(watch_paths(
            connection.clone(),
            self.activity.clone(),
            self.relay_only_after,
        ));
        let declined = {
            let activity = self.activity.clone();
            tokio::spawn(watch_declined(
                connection.clone(),
                self.endpoint.clone(),
                self.probe.clone(),
                move |declined| activity.local_network_declined(id, declined),
            ))
        };
        self.events
            .client_connected(|| ClientConnected {
                connection_id: id,
                endpoint_id: Some(connection.remote_id()),
            })
            .await
            .ok();
        // At most this many transfers of the connection at once: its next
        // stream is taken only once one ends (ADR 0031).
        let places = Arc::new(Semaphore::new(limits::TRANSFERS_PER_CONNECTION));
        let asks = Arc::new(limits::Asks::new(std::time::Instant::now()));
        loop {
            let Ok(place) = places.clone().acquire_owned().await else {
                break;
            };
            let Ok((writer, reader)) = connection.accept_bi().await else {
                break;
            };
            let handler = self.clone();
            let asks = asks.clone();
            tokio::spawn(async move {
                serve_stream(handler, id, &asks, writer, reader).await;
                drop(place);
            });
        }
        watch.abort();
        declined.abort();
        // Ended before the connection is forgotten: a last "declined" told
        // while it stopped would otherwise stay.
        let _ = declined.await;
        self.events
            .connection_closed(|| ConnectionClosed { connection_id: id })
            .await
            .ok();
        self.activity.closed(id);
        self.demand.send(Demand::Closed { connection: id }).ok();
        Ok(())
    }
}

/// One request on one stream.
async fn serve_stream(
    handler: BlobsHandler,
    connection: u64,
    asks: &limits::Asks,
    mut writer: SendStream,
    mut reader: iroh::endpoint::RecvStream,
) {
    let request = match limits::read_request(&mut reader).await {
        Some(Request::Get(request)) => request,
        Some(_) => {
            tracing::debug!("refused a request that is not a get");
            refuse(&mut writer, ERR_PERMISSION);
            return;
        }
        None => {
            tracing::debug!("a request that is unreadable, too long or too slow");
            refuse(&mut writer, ERR_PERMISSION);
            return;
        }
    };
    let decision = handler
        .serving
        .decide(request.hash, request.ranges.is_blob());
    // A hint for the owner's keeper, so many a second per connection: a
    // reader asking again and again does not fill its queue (ADR 0031).
    if !matches!(decision, Decision::Refuse) && asks.take(std::time::Instant::now()) {
        handler
            .demand
            .send(Demand::Asked {
                connection,
                hash: *request.hash.as_bytes(),
                served: matches!(decision, Decision::Serve(_)),
            })
            .ok();
    }
    match decision {
        Decision::Serve(guard) => {
            // A file gone or not of its size (a disk unplugged, a file
            // renamed or cut short) is "not ready", before anything is sent.
            if let Some(probe) = guard.probe() {
                if !probe.can_send(request.hash).await {
                    drop(guard);
                    handler.activity.told_not_ready(connection);
                    refuse(&mut writer, ERR_LIMIT);
                    return;
                }
            }
            // The program serves all it may: asked again later, as if not
            // ready (ADR 0031).
            let Some(place) = limits::transfer_place() else {
                drop(guard);
                tracing::debug!("a transfer waits: the program serves all it may");
                refuse(&mut writer, ERR_LIMIT);
                return;
            };
            handler.activity.started(connection);
            let pair = StreamPair::new(
                connection,
                reader,
                limits::Watched(writer),
                handler.events.clone(),
            );
            if let Err(error) = handle_get(pair, guard.store(), request).await {
                tracing::debug!(%error, "a transfer ended early");
            }
            handler.activity.ended(connection);
            // The store may leave now.
            drop(guard);
            drop(place);
        }
        Decision::NotReady => {
            handler.activity.told_not_ready(connection);
            refuse(&mut writer, ERR_LIMIT);
        }
        Decision::Refuse => {
            tracing::debug!("refused a request for a blob that is not served");
            refuse(&mut writer, ERR_PERMISSION);
        }
    }
}

fn refuse(writer: &mut SendStream, code: iroh::endpoint::VarInt) {
    writer.reset(code).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gets_are_logged_and_the_rest_disabled() {
        let mask = event_mask();
        assert_eq!(mask.get, RequestMode::NotifyLog);
        assert_eq!(mask.get_many, RequestMode::Disabled);
        assert_eq!(mask.push, RequestMode::Disabled);
    }
}
