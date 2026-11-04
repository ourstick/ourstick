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

/// Connections one stick's owner takes at once on its blobs protocol. Each
/// costs QUIC state and two watching tasks; a stick's readers are at most
/// 200 (ADR 0015), and few are connected at once.
pub(super) const CONNECTIONS_PER_STICK: usize = 128;

// Two readers fetching all they may at once both fit.
const _: () = assert!(TRANSFERS_AT_ONCE >= 2 * TRANSFERS_PER_CONNECTION);

/// Streams a peer may have open at once on one connection of an owner.
const STREAMS_PER_CONNECTION: u32 = TRANSFERS_PER_CONNECTION as u32 + 1;

/// What a peer may send on one stream of an owner before it must wait: a
/// request is small.
const STREAM_WINDOW: u32 = 256 * 1024;

/// What a peer may send on all streams of one connection of an owner.
const CONNECTION_WINDOW: u32 = 1024 * 1024;

/// The QUIC limits of an owner's endpoint: it only takes requests.
pub(super) fn serving_transport() -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .max_concurrent_bidi_streams(VarInt::from_u32(STREAMS_PER_CONNECTION))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
        .receive_window(VarInt::from_u32(CONNECTION_WINDOW))
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

/// A transfer's stream that ends the transfer once the reader has taken
/// nothing for [`STALL`].
#[derive(Debug)]
pub(super) struct Watched(pub(super) SendStream);

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
        within(iroh_blobs::util::SendStream::send_bytes(&mut self.0, bytes)).await
    }

    async fn send(&mut self, buf: &[u8]) -> io::Result<()> {
        within(iroh_blobs::util::SendStream::send(&mut self.0, buf)).await
    }

    async fn sync(&mut self) -> io::Result<()> {
        within(iroh_blobs::util::SendStream::sync(&mut self.0)).await
    }

    fn reset(&mut self, code: VarInt) -> io::Result<()> {
        iroh_blobs::util::SendStream::reset(&mut self.0, code)
    }

    async fn stopped(&mut self) -> io::Result<Option<VarInt>> {
        iroh_blobs::util::SendStream::stopped(&mut self.0).await
    }

    fn id(&self) -> u64 {
        iroh_blobs::util::SendStream::id(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_blobs::protocol::{GetRequest, PushRequest};
    use iroh_blobs::Hash;

    fn written(request: &Request) -> Vec<u8> {
        postcard::to_allocvec(request).unwrap()
    }

    #[test]
    fn a_get_is_read_as_iroh_blobs_writes_it() {
        let get = Request::Get(GetRequest::blob(Hash::new(b"a")));
        let bytes = written(&get);
        assert!(bytes.len() < 100, "{} bytes", bytes.len());
        assert_eq!(parse_request(&bytes), Some(get));
        // The kind byte is iroh-blobs' own (`RequestType`): a get is 0.
        assert_eq!(bytes[0], 0);
    }

    #[test]
    fn other_kinds_are_read_as_such_and_garbage_is_not() {
        let push = Request::Push(PushRequest::new(
            Hash::new(b"a"),
            iroh_blobs::protocol::ChunkRangesSeq::root(),
        ));
        assert!(matches!(
            parse_request(&written(&push)),
            Some(Request::Push(_))
        ));
        let mut trailing = written(&Request::Get(GetRequest::blob(Hash::new(b"a"))));
        trailing.push(0);
        assert_eq!(parse_request(&trailing), None);
        assert_eq!(parse_request(&[]), None);
        assert_eq!(parse_request(&[200, 1, 2]), None);
    }

    #[tokio::test]
    async fn a_send_the_reader_never_takes_ends() {
        let never = std::future::pending::<io::Result<()>>();
        let ended = within_for(Duration::from_millis(20), never).await;
        assert_eq!(ended.unwrap_err().kind(), io::ErrorKind::TimedOut);
        let sent = within_for(Duration::from_millis(20), async { Ok(7) }).await;
        assert_eq!(sent.unwrap(), 7);
    }

    #[test]
    fn the_program_wide_places_run_out_and_come_back() {
        let semaphore = Arc::new(Semaphore::new(2));
        let first = semaphore.clone().try_acquire_owned().unwrap();
        let _second = semaphore.clone().try_acquire_owned().unwrap();
        assert!(semaphore.clone().try_acquire_owned().is_err());
        drop(first);
        assert!(semaphore.try_acquire_owned().is_ok());
    }
}
