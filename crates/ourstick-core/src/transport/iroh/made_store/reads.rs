//! Transfers read their files through the disks' turns (ADR 0017,
//! amendment 1): one [`SharedReader`] per disk for all the stick's
//! transfers.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::blob::made::{FileReads, MadeBlob};
use crate::disk::turns::{DiskId, DiskTurns, SharedReader};

/// The stick's transfers' readers, by disk.
#[derive(Clone)]
pub(in crate::transport::iroh) struct Reads {
    turns: Arc<DiskTurns>,
    readers: Arc<Mutex<HashMap<DiskId, Arc<SharedReader>>>>,
    /// Set when the store shuts down: reads waiting for a turn leave.
    stopped: Arc<AtomicBool>,
}

impl Reads {
    pub(in crate::transport::iroh) fn new(turns: Arc<DiskTurns>) -> Self {
        Self {
            turns,
            readers: Arc::default(),
            stopped: Arc::default(),
        }
    }

    /// `blob`, of the file at `path`, read through its disk's turns.
    pub(super) fn through_turns(&self, blob: MadeBlob, path: &Path) -> io::Result<MadeBlob> {
        let disk = DiskId::of_file(path, &blob.metadata()?);
        let reader = self
            .readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(disk.clone())
            .or_insert_with(|| Arc::new(SharedReader::new(self.turns.get(disk))))
            .clone();
        Ok(blob.with_reads(Arc::new(TurnReads {
            reader,
            stopped: self.stopped.clone(),
        })))
    }

    /// Whether a disk the stick's transfers read gives way now.
    pub(in crate::transport::iroh) fn giving_way(&self) -> bool {
        self.readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .any(|reader| reader.giving_way())
    }

    /// No read waits for a turn any more; the turns held are given back.
    pub(in crate::transport::iroh) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

struct TurnReads {
    reader: Arc<SharedReader>,
    stopped: Arc<AtomicBool>,
}

impl FileReads for TurnReads {
    fn read(&self, begins: bool, read: &mut dyn FnMut() -> io::Result<u64>) -> io::Result<()> {
        let go_on = || {
            if self.stopped.load(Ordering::Relaxed) {
                Err(io::Error::other("sharing stopped"))
            } else {
                Ok(())
            }
        };
        self.reader
            .read(begins, &go_on, || read().map(|bytes| ((), bytes)))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::cipher::{ChaChaCipher, Cipher, StickKey};
    use crate::disk::turns::{DiskReader, SystemTimer};

    #[test]
    fn a_read_waiting_for_a_turn_leaves_when_the_store_stops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, vec![1u8; 100]).unwrap();
        let turns = Arc::new(DiskTurns::new(Arc::new(SystemTimer::default())));
        // Another reader holds the file's disk, reading.
        let disk = DiskId::of_file(&path, &std::fs::metadata(&path).unwrap());
        let mut holder = DiskReader::new(turns.get(disk), false, || Ok(()));
        let mut timed = holder.open(std::io::Cursor::new(vec![0u8; 10]), 10);
        timed.read_exact(&mut [0u8; 1]).unwrap();
        let reads = Reads::new(turns.clone());
        let cipher: Arc<dyn Cipher> = Arc::new(ChaChaCipher::new(&StickKey::from_bytes([1; 32])));
        let blob = MadeBlob::open(&path, cipher, 100, &[0; 32]).unwrap();
        let blob = reads.through_turns(blob, &path).unwrap();
        let waiting = std::thread::spawn(move || {
            let started = Instant::now();
            // Past the header: a chunk, which needs the file.
            let read = blob.read_at(blob.len() - 1, &mut [0u8; 1]);
            (read.is_err(), started.elapsed())
        });
        std::thread::sleep(Duration::from_millis(200));
        assert!(!waiting.is_finished(), "waits for the turn");
        reads.stop();
        let (failed, waited) = waiting.join().unwrap();
        assert!(failed);
        assert!(waited < Duration::from_secs(2), "{waited:?}");
    }
}
