//! What one peer may hold of an owner (ADR 0031): anyone who knows a node
//! id can connect, and anyone with a link can ask for its blobs, so nothing
//! a peer asks for may hold the owner's memory, files or transfers for
//! longer, or in greater number, than these bounds.

use std::future::Future;
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use iroh::endpoint::{QuicTransportConfig, RecvStream, SendStream, VarInt};
use iroh_blobs::protocol::Request;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The longest request read. A get of one blob is under 100 bytes; the
/// ranges of a resume add a few bytes per gap.
pub(super) const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// How long a peer has to send its request once it opened a stream, or to
/// open its stream once connected (record and report protocols).
pub(super) const REQUEST_WAIT: Duration = Duration::from_secs(10);

/// How long a transfer may go without the reader taking anything: it is
/// then ended, and the file it held is closed. A reader that comes back
/// later asks again and resumes.
pub(super) const STALL: Duration = Duration::from_secs(120);

/// Transfers served at once on one connection: a reader fetches at most 32
/// files at once (ADR 0016). Its other streams wait, unread.
pub(super) const TRANSFERS_PER_CONNECTION: usize = 32;

/// Transfers served at once by the whole program, all sticks together: each
/// holds a file and a hash tree open (macOS gives an app 256 files). One
/// more is refused as "not ready", and the reader asks again later.
pub(super) const TRANSFERS_AT_ONCE: usize = 64;

/// Connections one stick's owner keeps at once on its blobs protocol. Each
/// costs QUIC state and two watching tasks; a stick's readers are at most
/// 200 (ADR 0015), and few are connected at once. One more makes room by
/// closing the connection idle the longest (with no transfer), so that
/// connections that ask nothing, which anyone knowing the node id can
/// open, never keep a reader out.
pub(super) const CONNECTIONS_PER_STICK: usize = 128;

/// The connections of one stick's blobs protocol, with what each does.
#[derive(Debug)]
pub(super) struct Connections<C>(std::sync::Mutex<std::collections::HashMap<u64, Held<C>>>);

#[derive(Debug)]
struct Held<C> {
    connection: C,
    transfers: usize,
    last_used: std::time::Instant,
}

impl<C> Default for Connections<C> {
    fn default() -> Self {
        Self(std::sync::Mutex::new(std::collections::HashMap::new()))
    }
}

impl<C: Clone> Connections<C> {
    fn held(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, Held<C>>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes the connection `id`. `Ok` with the connection to close to make
    /// room, if one had to go; `Err` if every one has a transfer.
    pub(super) fn admit(
        &self,
        id: u64,
        connection: C,
        now: std::time::Instant,
    ) -> Result<Option<C>, ()> {
        let mut held = self.held();
        let mut evicted = None;
        if held.len() >= CONNECTIONS_PER_STICK {
            let idle = held
                .iter()
                .filter(|(_, held)| held.transfers == 0)
                .min_by_key(|(_, held)| held.last_used)
                .map(|(id, _)| *id)
                .ok_or(())?;
            evicted = held.remove(&idle).map(|held| held.connection);
        }
        held.insert(
            id,
            Held {
                connection,
                transfers: 0,
                last_used: now,
            },
        );
        Ok(evicted)
    }

    /// A transfer of `id` starts (`true`) or ends.
    pub(super) fn transfer(&self, id: u64, starts: bool, now: std::time::Instant) {
        if let Some(held) = self.held().get_mut(&id) {
            held.transfers = if starts {
                held.transfers + 1
            } else {
                held.transfers.saturating_sub(1)
            };
            held.last_used = now;
        }
    }

    pub(super) fn closed(&self, id: u64) {
        self.held().remove(&id);
    }
}

// Two readers fetching all they may at once both fit.
const _: () = assert!(TRANSFERS_AT_ONCE >= 2 * TRANSFERS_PER_CONNECTION);

/// Streams a peer may have open at once on one connection of an owner.
const STREAMS_PER_CONNECTION: u32 = TRANSFERS_PER_CONNECTION as u32 + 1;

/// What a peer may send on one stream of an owner before it must wait: a
/// request is small.
const STREAM_WINDOW: u32 = 256 * 1024;

/// What a peer may send on all streams of one connection of an owner.
const CONNECTION_WINDOW: u32 = 1024 * 1024;

/// What a stick's "not ready" asks may tell the owner's keeper (ADR 0031): a steady
/// rate, and a burst of [`ASKS_BURST`].
const ASKS_PER_SECOND: f64 = 64.0;
const ASKS_BURST: f64 = 256.0;

/// The asks of one stick that the owner's keeper hears of.
#[derive(Debug)]
pub(super) struct Asks(std::sync::Mutex<(f64, std::time::Instant)>);

impl Asks {
    pub(super) fn new(now: std::time::Instant) -> Self {
        Self(std::sync::Mutex::new((ASKS_BURST, now)))
    }

    /// Whether an ask at `now` is told.
    pub(super) fn take(&self, now: std::time::Instant) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (tokens, at) = &mut *state;
        let elapsed = now.saturating_duration_since(*at).as_secs_f64();
        *tokens = (*tokens + elapsed * ASKS_PER_SECOND).min(ASKS_BURST);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The QUIC limits of an owner's endpoint: it only takes requests.
pub(super) fn serving_transport() -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .max_concurrent_bidi_streams(VarInt::from_u32(STREAMS_PER_CONNECTION))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
        .receive_window(VarInt::from_u32(CONNECTION_WINDOW))
        // Nothing is ever read from datagrams: none is kept.
        .datagram_receive_buffer_size(None)
        .build()
}

/// The program's transfer places, shared by every stick's owner: the
/// limit is of the program's open files, which all sticks share.
fn program_transfers() -> &'static Arc<Semaphore> {
    static TRANSFERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    TRANSFERS.get_or_init(|| Arc::new(Semaphore::new(TRANSFERS_AT_ONCE)))
}

/// A place to serve one transfer in, or `None` if the program serves all it
/// may now.
pub(super) fn transfer_place() -> Option<OwnedSemaphorePermit> {
    program_transfers().clone().try_acquire_owned().ok()
}

/// Reads one blobs request: at most [`MAX_REQUEST_BYTES`], within
/// [`REQUEST_WAIT`]. `None` for anything else.
pub(super) async fn read_request(reader: &mut RecvStream) -> Option<Request> {
    let bytes = tokio::time::timeout(REQUEST_WAIT, reader.read_to_end(MAX_REQUEST_BYTES))
        .await
        .ok()?
        .ok()?;
    parse_request(&bytes)
}

/// A request as iroh-blobs writes it: its kind, then its body, nothing after.
fn parse_request(bytes: &[u8]) -> Option<Request> {
    match postcard::take_from_bytes::<Request>(bytes) {
        Ok((request, [])) => Some(request),
        _ => None,
    }
}

/// The least a reader takes in each [`STALL`] of a transfer: one taking
/// less (a trickle that keeps each write going) holds the transfer's place
/// for nothing, and the transfer ends.
pub(super) const LEAST_PER_STALL: u64 = 64 * 1024;

/// A transfer's stream that ends the transfer once the reader has taken
/// nothing for [`STALL`], or less than [`LEAST_PER_STALL`] in one.
#[derive(Debug)]
pub(super) struct Watched {
    stream: SendStream,
    pace: Pace,
}

impl Watched {
    pub(super) fn new(stream: SendStream) -> Self {
        Self {
            stream,
            pace: Pace::new(std::time::Instant::now()),
        }
    }
}

/// How much a reader took since when.
#[derive(Debug)]
struct Pace {
    since: std::time::Instant,
    taken: u64,
}

impl Pace {
    fn new(now: std::time::Instant) -> Self {
        Self {
            since: now,
            taken: 0,
        }
    }

    /// `bytes` more were taken at `now`: an error once a whole [`STALL`]
    /// passed with less than [`LEAST_PER_STALL`] taken.
    fn took(&mut self, bytes: u64, now: std::time::Instant) -> io::Result<()> {
        self.taken = self.taken.saturating_add(bytes);
        if now.saturating_duration_since(self.since) < STALL {
            return Ok(());
        }
        if self.taken < LEAST_PER_STALL {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the reader takes too little",
            ));
        }
        *self = Self::new(now);
        Ok(())
    }
}

async fn within<T>(sending: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    within_for(STALL, sending).await
}

async fn within_for<T>(
    limit: Duration,
    sending: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    tokio::time::timeout(limit, sending)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the reader took nothing"))?
}

impl iroh_blobs::util::SendStream for Watched {
    async fn send_bytes(&mut self, bytes: Bytes) -> io::Result<()> {
        let len = bytes.len() as u64;
        within(iroh_blobs::util::SendStream::send_bytes(
            &mut self.stream,
            bytes,
        ))
        .await?;
        self.pace.took(len, std::time::Instant::now())
    }

    async fn send(&mut self, buf: &[u8]) -> io::Result<()> {
        within(iroh_blobs::util::SendStream::send(&mut self.stream, buf)).await?;
        self.pace.took(buf.len() as u64, std::time::Instant::now())
    }

    async fn sync(&mut self) -> io::Result<()> {
        within(iroh_blobs::util::SendStream::sync(&mut self.stream)).await
    }

    fn reset(&mut self, code: VarInt) -> io::Result<()> {
        iroh_blobs::util::SendStream::reset(&mut self.stream, code)
    }

    async fn stopped(&mut self) -> io::Result<Option<VarInt>> {
        iroh_blobs::util::SendStream::stopped(&mut self.stream).await
    }

    fn id(&self) -> u64 {
        iroh_blobs::util::SendStream::id(&self.stream)
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
