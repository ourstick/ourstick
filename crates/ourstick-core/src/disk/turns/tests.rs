use super::*;
use std::io::{Read, Seek, SeekFrom};

const MB: usize = 1 << 20;

/// Time moves only when a fake file is read or someone sleeps.
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

/// A file of `len` bytes whose reads take `first` a megabyte, or `second`
/// a megabyte once it was read and sought back to its start.
struct FakeFile {
    timer: Arc<FakeTimer>,
    len: u64,
    at: u64,
    first: Duration,
    second: Duration,
    rewound: bool,
}

impl FakeFile {
    fn new(timer: &Arc<FakeTimer>, len: usize, first_ms_per_mb: u64) -> Self {
        Self {
            timer: timer.clone(),
            len: len as u64,
            at: 0,
            first: Duration::from_millis(first_ms_per_mb),
            second: Duration::from_millis(first_ms_per_mb),
            rewound: false,
        }
    }
}

impl Read for FakeFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = (buffer.len() as u64).min(self.len - self.at);
        self.at += read;
        let per_mb = if self.rewound {
            self.second
        } else {
            self.first
        };
        self.timer.sleep(per_mb.mul_f64(read as f64 / MB as f64));
        Ok(read as usize)
    }
}

impl Seek for FakeFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if let SeekFrom::Start(at) = position {
            self.rewound |= at == 0 && self.at > 0;
            self.at = at;
        }
        Ok(self.at)
    }
}

fn read_through(reader: &mut DiskReader, file: FakeFile) {
    let len = file.len;
    let mut timed = reader.open(file, len);
    std::io::copy(&mut timed, &mut std::io::sink()).unwrap();
}

fn setup() -> (Arc<FakeTimer>, DiskTurns) {
    let timer = Arc::new(FakeTimer::default());
    let disks = DiskTurns::new(timer.clone());
    (timer, disks)
}

fn go_on() -> io::Result<()> {
    Ok(())
}

#[test]
fn a_big_file_is_judged_window_by_window() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    // 128 MB at 8 ms a MB (125 MB/s): two windows, fast.
    read_through(&mut reader, FakeFile::new(&timer, 192 * MB, 8));
    assert!(!disk.giving_way());
    // Then, inside one file, at 40 ms a MB: after two slow windows it gives
    // way, before the file ends.
    let before = timer.elapsed();
    read_through(&mut reader, FakeFile::new(&timer, 256 * MB, 40));
    reader.end_window();
    assert!(disk.giving_way());
    // The pauses were taken: more time passed than the reading alone.
    let reading = Duration::from_millis(256 * 40);
    assert!(timer.elapsed() - before > reading);
}

#[test]
fn only_the_first_read_of_a_file_is_judged() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    read_through(&mut reader, FakeFile::new(&timer, 192 * MB, 8));
    // Read twice, as `encrypt_blob` does: a slow second read (the file was
    // not in memory) is not someone else.
    for _ in 0..4 {
        let mut file = FakeFile::new(&timer, 64 * MB, 8);
        file.second = Duration::from_millis(80);
        let mut timed = reader.open(file, 64 * MB as u64);
        std::io::copy(&mut timed, &mut std::io::sink()).unwrap();
        timed.seek(SeekFrom::Start(0)).unwrap();
        std::io::copy(&mut timed, &mut std::io::sink()).unwrap();
    }
    reader.end_window();
    assert!(!disk.giving_way());
}

#[test]
fn small_files_close_a_window_every_64() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    // 192 files of 100 KB at 100 ms a MB (10 ms each): three windows, the
    // measure.
    for _ in 0..192 {
        read_through(&mut reader, FakeFile::new(&timer, 100 * 1024, 100));
    }
    // Then four times slower, 128 files: two windows, giving way.
    for _ in 0..128 {
        read_through(&mut reader, FakeFile::new(&timer, 100 * 1024, 400));
    }
    // The last window is given back at the end of a part.
    reader.end_window();
    assert!(disk.giving_way());
    assert_eq!(lock(&disk.state).windows, 5);
    // Exactly 64 files are one window, the 65th begins the next.
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    for _ in 0..64 {
        read_through(&mut reader, FakeFile::new(&timer, 100 * 1024, 100));
    }
    reader.end_window();
    assert_eq!(lock(&disk.state).windows, 1);
    for _ in 0..65 {
        read_through(&mut reader, FakeFile::new(&timer, 100 * 1024, 100));
    }
    reader.end_window();
    assert_eq!(lock(&disk.state).windows, 3);
}

#[test]
fn the_pause_is_the_disks_and_the_next_turn_waits_for_it() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut first = DiskReader::new(disk.clone(), false, go_on);
    read_through(&mut first, FakeFile::new(&timer, 192 * MB, 8));
    // Exactly two slow windows, the turn given back at the end of a part,
    // before the file's end is read.
    let mut timed = first.open(FakeFile::new(&timer, 128 * MB, 40), 128 * MB as u64);
    let mut buffer = vec![0; MB];
    for _ in 0..128 {
        timed.read_exact(&mut buffer).unwrap();
    }
    drop(timed);
    first.end_window();
    assert!(disk.giving_way());
    // Another reader of the same disk starts only after the pause.
    let ended = timer.elapsed();
    let mut second = DiskReader::new(disk.clone(), false, go_on);
    let mut timed = second.open(FakeFile::new(&timer, MB, 40), MB as u64);
    timed.read_exact(&mut buffer).unwrap();
    let window = Duration::from_millis(64 * 40);
    assert!(timer.elapsed() - ended >= window * 9);
    // A turn that reads nothing (a file's end) leaves the disk's pause as
    // it was.
    drop(timed);
    second.end_window();
    let pause = lock(&disk.state).not_before;
    let mut third = DiskReader::new(disk.clone(), false, go_on);
    let mut timed = third.open(FakeFile::new(&timer, 0, 40), u64::MAX);
    assert_eq!(timed.read(&mut buffer).unwrap(), 0);
    drop(timed);
    third.end_window();
    assert_eq!(lock(&disk.state).not_before, pause);
    assert!(disk.giving_way(), "an empty turn is no fast window");
}

#[test]
fn reading_a_files_end_waits_for_no_pause() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    read_through(&mut reader, FakeFile::new(&timer, 192 * MB, 8));
    // The second slow window closes exactly at the file's end: the read
    // that finds the end does not wait for the pause that follows.
    let before = timer.elapsed();
    read_through(&mut reader, FakeFile::new(&timer, 128 * MB, 40));
    assert!(disk.giving_way());
    let reading = Duration::from_millis(128 * 40);
    // Only the pause after the first slow window (none: it was one alone)
    // is in it, not the one after the second.
    assert!(timer.elapsed() - before < reading + Duration::from_secs(1));
}

#[test]
fn a_reader_told_to_stop_during_a_pause_stops_and_frees_the_disk() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    read_through(&mut reader, FakeFile::new(&timer, 192 * MB, 8));
    let mut timed = reader.open(FakeFile::new(&timer, 128 * MB, 40), 128 * MB as u64);
    let mut buffer = vec![0; MB];
    for _ in 0..128 {
        timed.read_exact(&mut buffer).unwrap();
    }
    drop(timed);
    reader.end_window();
    assert!(disk.giving_way());
    // Stopped (or its drive gone) after the first slice of the pause.
    let asked = Arc::new(Mutex::new(0));
    let counted = asked.clone();
    let mut stopping = DiskReader::new(disk.clone(), false, move || {
        let mut asked = lock(&counted);
        *asked += 1;
        if *asked > 1 {
            Err(io::Error::other("stopped"))
        } else {
            Ok(())
        }
    });
    let mut timed = stopping.open(FakeFile::new(&timer, MB, 8), MB as u64);
    let error = std::io::copy(&mut timed, &mut std::io::sink()).unwrap_err();
    assert_eq!(error.to_string(), "stopped");
    assert_eq!(*lock(&asked), 2);
    // The disk is free for others.
    let mut other = DiskReader::new(disk, false, go_on);
    read_through(&mut other, FakeFile::new(&timer, MB, 8));
}

#[test]
fn readers_on_threads_take_turns_without_waiting_forever() {
    let disks = DiskTurns::new(Arc::new(SystemTimer::default()));
    let disk = disks.get(DiskId::Unknown);
    let threads: Vec<_> = (0..4)
        .map(|index| {
            let disk = disk.clone();
            std::thread::spawn(move || {
                let mut reader = DiskReader::new(disk, index == 0, go_on);
                for _ in 0..50 {
                    let file = std::io::Cursor::new(vec![0u8; 100_000]);
                    let mut timed = reader.open(file, 100_000);
                    std::io::copy(&mut timed, &mut std::io::sink()).unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn folders_on_one_filesystem_are_one_disk() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("a")).unwrap();
    let id = DiskId::of(folder.path());
    assert_eq!(DiskId::of(&folder.path().join("a")), id);
    assert_ne!(id, DiskId::Unknown);
    let (_, disks) = setup();
    assert!(Arc::ptr_eq(
        &disks.of(folder.path()),
        &disks.of(&folder.path().join("a"))
    ));
}

#[test]
fn a_reader_stopped_while_queued_leaves_the_queue_at_once() {
    let disks = DiskTurns::new(Arc::new(SystemTimer::default()));
    let disk = disks.get(DiskId::Unknown);
    // One reader holds the turn (a window open)…
    let mut holder = DiskReader::new(disk.clone(), false, go_on);
    let mut timed = holder.open(std::io::Cursor::new(vec![0u8; 10]), 10);
    timed.read_exact(&mut [0; 5]).unwrap();
    // …another waits for it, and is stopped while it waits.
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let told = stopped.clone();
    let waiter = std::thread::spawn(move || {
        let mut reader = DiskReader::new(disk, false, move || {
            if told.load(std::sync::atomic::Ordering::SeqCst) {
                Err(io::Error::other("stopped"))
            } else {
                Ok(())
            }
        });
        let mut timed = reader.open(std::io::Cursor::new(vec![0u8; 10]), 10);
        let started = Instant::now();
        let read = timed.read(&mut [0; 10]);
        (read.map_err(|error| error.to_string()), started.elapsed())
    });
    std::thread::sleep(Duration::from_millis(200));
    stopped.store(true, std::sync::atomic::Ordering::SeqCst);
    let (read, waited) = waiter.join().unwrap();
    assert_eq!(read, Err("stopped".to_owned()));
    // Asked every half second: gone well before the holder gives its turn.
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[test]
fn a_file_grown_since_its_length_was_taken_is_read_in_turns() {
    let (timer, disks) = setup();
    let disk = disks.get(DiskId::Unknown);
    let mut reader = DiskReader::new(disk.clone(), false, go_on);
    read_through(&mut reader, FakeFile::new(&timer, 192 * MB, 8));
    // Its length was 1 MB when it was opened; it is 128 MB now, and slow.
    let mut timed = reader.open(FakeFile::new(&timer, 128 * MB, 40), MB as u64);
    std::io::copy(&mut timed, &mut std::io::sink()).unwrap();
    drop(timed);
    reader.end_window();
    // Read past its first length in turns, and judged: giving way.
    assert!(disk.giving_way());
}
