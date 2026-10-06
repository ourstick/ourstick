use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::disk::turns::{DiskReader, Timer, IDLE_GRACE};

/// Time moves only when a test says.
#[derive(Default)]
struct FakeTimer(Mutex<Duration>);

impl Timer for FakeTimer {
    fn elapsed(&self) -> Duration {
        *lock(&self.0)
    }

    fn sleep(&self, duration: Duration) {
        *lock(&self.0) += duration;
    }
}

fn setup() -> (Arc<FakeTimer>, Arc<Disk>) {
    let timer = Arc::new(FakeTimer::default());
    let disk = Arc::new(Disk::new(timer.clone()));
    (timer, disk)
}

fn go_on() -> io::Result<()> {
    Ok(())
}

/// A read of `bytes` taking `ms` of fake time.
fn read(reader: &SharedReader, timer: &FakeTimer, begins: bool, bytes: u64, ms: u64) {
    reader
        .read(begins, &go_on, || {
            timer.sleep(Duration::from_millis(ms));
            Ok(((), bytes))
        })
        .unwrap();
}

/// Reads a byte of a file through `reader`, which then holds its turn.
fn read_a_byte(reader: &mut DiskReader) {
    use std::io::Read;
    let mut timed = reader.open(std::io::Cursor::new(vec![0u8; 10]), 10);
    timed.read_exact(&mut [0u8; 1]).unwrap();
}

fn held(disk: &Disk) -> bool {
    lock(&disk.state).held
}

fn windows(disk: &Disk) -> u64 {
    lock(&disk.state).windows
}

#[test]
fn reads_share_one_turn_and_a_full_window_is_judged_once() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk.clone());
    for _ in 0..63 {
        read(&reader, &timer, true, 4096, 1);
        // Nobody waits: the turn is kept, idle, and nothing is judged.
        assert!(held(&disk));
    }
    assert_eq!(windows(&disk), 0);
    assert_eq!(lock(&disk.state).turn, 1, "one turn for all");
    read(&reader, &timer, true, 4096, 1);
    // 64 files begun: judged, the turn given back.
    assert_eq!(windows(&disk), 1);
    assert!(!held(&disk));
}

#[test]
fn a_window_takes_the_wall_time_of_reads_at_once() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk);
    reader.join(&go_on).unwrap();
    reader.join(&go_on).unwrap();
    timer.sleep(Duration::from_millis(100));
    reader.leave(1000, true);
    reader.leave(1000, true);
    let state = lock(&reader.state);
    assert_eq!(state.window.took, Duration::from_millis(100));
    assert_eq!(state.window.timed_files, 2);
    assert_eq!(state.window.bytes, 2000);
}

#[test]
fn an_idle_turn_goes_to_a_reader_that_waits() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk.clone());
    read(&reader, &timer, true, 4096, 1);
    assert!(held(&disk));
    // A catalogue asks while the transfers wait on the network.
    let waiting = {
        let disk = disk.clone();
        std::thread::spawn(move || {
            let mut catalogue = DiskReader::new(disk, false, go_on);
            read_a_byte(&mut catalogue);
            catalogue.end_window();
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiting.is_finished(), "not before the grace");
    timer.sleep(IDLE_GRACE);
    waiting.join().unwrap();
    // The transfers take a turn again at their next read.
    read(&reader, &timer, true, 4096, 1);
    assert_eq!(lock(&disk.state).turn, 3);
}

#[test]
fn with_someone_waiting_a_turn_is_given_back_after_its_share() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk.clone());
    // Someone waits: an entry in the queue.
    lock(&disk.state).waiting.push_back((99, false));
    lock(&disk.state).next_ticket = 100;
    read(&reader, &timer, true, 4096, 1);
    assert!(held(&disk), "a turn too young to give back");
    read(&reader, &timer, false, 4096, 300);
    assert!(!held(&disk), "held past its time: given back");
    assert_eq!(windows(&disk), 0, "not judged before it is full");
}

#[test]
fn a_failed_read_leaves_no_read_in_flight() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk.clone());
    let failed: io::Result<()> = reader.read(true, &go_on, || Err(io::Error::other("gone")));
    assert!(failed.is_err());
    assert_eq!(lock(&reader.state).in_flight, 0);
    read(&reader, &timer, true, 4096, 1);
    assert_eq!(lock(&reader.state).window.timed_files, 1);
}

#[test]
fn a_read_told_to_stop_while_waiting_leaves_at_once() {
    let (_timer, disk) = setup();
    // Another reader holds the disk, and is not idle.
    let mut holder = DiskReader::new(disk.clone(), false, go_on);
    read_a_byte(&mut holder);
    let reader = Arc::new(SharedReader::new(disk.clone()));
    let stop = Arc::new(AtomicBool::new(false));
    let waiter = {
        let reader = reader.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let go_on = || {
                if stop.load(Ordering::SeqCst) {
                    Err(io::Error::other("stopped"))
                } else {
                    Ok(())
                }
            };
            reader.read(true, &go_on, || Ok(((), 1)))
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::SeqCst);
    let read = waiter.join().unwrap();
    assert_eq!(read.unwrap_err().to_string(), "stopped");
    let state = lock(&reader.state);
    assert!(!state.taking);
    assert_eq!(state.in_flight, 0);
    assert!(state.turn.is_none());
    drop(state);
    holder.end_window();
    assert!(lock(&disk.state).waiting.is_empty());
}

#[test]
fn a_dropped_reader_gives_its_turn_back() {
    let (timer, disk) = setup();
    let reader = SharedReader::new(disk.clone());
    read(&reader, &timer, true, 4096, 1);
    assert!(held(&disk));
    drop(reader);
    assert!(!held(&disk));
}
