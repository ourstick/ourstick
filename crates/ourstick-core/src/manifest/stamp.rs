//! A cheap fingerprint of a file on disk, to tell whether it changed since it
//! was last encrypted without reading it. A heuristic with a known limit (see
//! "Known limits" in `docs/protocol.md`): a change that keeps the size, mtime
//! and ctime is not seen. `share --rescan` reads every file instead.

use std::fs::Metadata;
use std::time::{SystemTime, UNIX_EPOCH};

/// Size and times of a file, in nanoseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileStamp {
    pub size: u64,
    pub mtime_ns: u64,
    /// The inode change time on Unix, which every write updates and which
    /// cannot be set back by the user (unlike mtime). 0 elsewhere.
    pub ctime_ns: u64,
}

fn nanos_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

impl FileStamp {
    pub fn of(metadata: &Metadata) -> Self {
        Self {
            size: metadata.len(),
            mtime_ns: metadata.modified().map(nanos_since_epoch).unwrap_or(0),
            ctime_ns: change_time_ns(metadata),
        }
    }
}

#[cfg(unix)]
fn change_time_ns(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let seconds = u64::try_from(metadata.ctime()).unwrap_or(0);
    let nanos = u64::try_from(metadata.ctime_nsec()).unwrap_or(0);
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

#[cfg(not(unix))]
fn change_time_ns(_metadata: &Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_changes_the_stamp_even_with_the_same_size_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"aaaa").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let before = FileStamp::of(&std::fs::metadata(&path).unwrap());
        assert_eq!(before.size, 4);
        assert!(before.mtime_ns > 0);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"bbbb").unwrap();
        file.set_modified(mtime).unwrap();
        let after = FileStamp::of(&std::fs::metadata(&path).unwrap());
        assert_eq!((after.size, after.mtime_ns), (before.size, before.mtime_ns));
        if cfg!(unix) {
            assert_ne!(after.ctime_ns, before.ctime_ns, "ctime shows the write");
        }
    }
}
