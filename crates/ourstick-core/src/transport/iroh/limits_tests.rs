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

#[test]
fn a_trickle_ends_the_transfer_and_a_slow_reader_does_not() {
    let start = std::time::Instant::now();
    let mut trickle = Pace::new(start);
    for step in 1..=12u64 {
        let now = start + Duration::from_secs(step * 10);
        let took = trickle.took(1024, now);
        assert_eq!(took.is_err(), now - start >= STALL, "{step}");
        if took.is_err() {
            break;
        }
    }
    // 1 KiB a second, over a slow link: well above the least.
    let mut slow = Pace::new(start);
    for second in 1..=600u64 {
        slow.took(1024, start + Duration::from_secs(second))
            .unwrap();
    }
}

#[test]
fn a_full_stick_lets_a_new_connection_in_by_closing_the_longest_idle() {
    let start = std::time::Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    let connections = Connections::<u64>::default();
    for id in 0..CONNECTIONS_PER_STICK as u64 {
        assert_eq!(connections.admit(id, id, at(id)), Ok(None));
    }
    // 0 is busy, 1 was idle the longest: 1 goes.
    connections.transfer(0, true, at(500));
    assert_eq!(connections.admit(1000, 1000, at(600)), Ok(Some(1)));
    // Every one busy: the new one is refused.
    for id in (2..CONNECTIONS_PER_STICK as u64).chain([1000]) {
        connections.transfer(id, true, at(700));
    }
    assert_eq!(connections.admit(1001, 1001, at(800)), Err(()));
    connections.transfer(0, false, at(900));
    assert_eq!(connections.admit(1001, 1001, at(901)), Ok(Some(0)));
    connections.closed(1001);
    assert_eq!(connections.admit(1002, 1002, at(902)), Ok(None));
}

#[test]
fn a_connections_asks_are_told_at_a_steady_rate() {
    let start = std::time::Instant::now();
    let asks = Asks::new(start);
    let told = (0..1000).filter(|_| asks.take(start)).count();
    assert_eq!(told, ASKS_BURST as usize);
    // A second later, a second's worth more.
    let later = start + Duration::from_secs(1);
    let more = (0..1000).filter(|_| asks.take(later)).count();
    assert_eq!(more, ASKS_PER_SECOND as usize);
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
