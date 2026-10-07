//! What the kernel counts for a disk (ADR 0013, amendment 1): operations
//! and bytes, read and written, by every program. Linux only, from
//! `/sys/dev/block/MAJOR:MINOR/stat`, followed down to the physical disk;
//! elsewhere nothing, and the disk is judged by timing alone.

use std::path::{Path, PathBuf};

use super::turns::DiskId;

/// A disk's counters at one moment, since the system started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Reads and writes completed.
    pub ops: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
}

impl Counters {
    /// What happened between `self` and the later `then` (nothing if the
    /// counters went back: a device that went and came again).
    pub fn until(self, then: Counters) -> Counters {
        Counters {
            ops: then.ops.saturating_sub(self.ops),
            read_bytes: then.read_bytes.saturating_sub(self.read_bytes),
            written_bytes: then.written_bytes.saturating_sub(self.written_bytes),
        }
    }

    pub fn add(&mut self, more: Counters) {
        self.ops = self.ops.saturating_add(more.ops);
        self.read_bytes = self.read_bytes.saturating_add(more.read_bytes);
        self.written_bytes = self.written_bytes.saturating_add(more.written_bytes);
    }
}

/// Where a disk's counters come from: the kernel, or a fake in tests.
pub trait DiskCounters: Send + Sync {
    /// The counters of the physical disk under `disk` now, if known. Blocking
    /// (reads a small file).
    fn read(&self, disk: &DiskId) -> Option<Counters>;
}

/// No counters: every disk judged by timing alone (tests that make reads
/// slow with a fake clock, which no kernel counts).
#[derive(Debug, Default)]
pub struct NoCounters;

impl DiskCounters for NoCounters {
    fn read(&self, _: &DiskId) -> Option<Counters> {
        None
    }
}

/// The kernel's counters (Linux); none elsewhere.
#[derive(Debug)]
pub struct SystemCounters {
    /// `/sys`, or a fake tree in tests.
    sys: PathBuf,
}

impl Default for SystemCounters {
    fn default() -> Self {
        Self {
            sys: PathBuf::from("/sys"),
        }
    }
}

impl SystemCounters {
    /// Counters read under `sys` instead of `/sys` (tests).
    pub fn under(sys: impl Into<PathBuf>) -> Self {
        Self { sys: sys.into() }
    }
}

impl DiskCounters for SystemCounters {
    fn read(&self, disk: &DiskId) -> Option<Counters> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let DiskId::Device(dev) = disk else {
            return None;
        };
        let (major, minor) = major_minor(*dev);
        // Anonymous devices (btrfs, overlay) have no line of their own.
        if major == 0 {
            return None;
        }
        let device = self.sys.join("dev/block").join(format!("{major}:{minor}"));
        let disk = physical(&device)?;
        parse_stat(&std::fs::read_to_string(disk.join("stat")).ok()?)
    }
}

/// glibc's layout of a `dev_t`: 12 bits of major and 20 of minor, split.
pub fn major_minor(dev: u64) -> (u64, u64) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major, minor)
}

/// The physical disk's folder in `/sys` for `device` (a link under
/// `/sys/dev/block`): down through a device-mapper layer while it has
/// exactly one device below it, then from a partition to its disk. Several
/// below (RAID, a volume over two disks): the device itself.
fn physical(device: &Path) -> Option<PathBuf> {
    let mut at = std::fs::canonicalize(device).ok()?;
    // Bounded: layers do not nest deeper than this.
    for _ in 0..8 {
        let below: Vec<PathBuf> = match std::fs::read_dir(at.join("slaves")) {
            Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
            Err(_) => Vec::new(),
        };
        match below.as_slice() {
            [only] => at = std::fs::canonicalize(only).ok()?,
            _ => break,
        }
    }
    // A partition has a `partition` file; its disk is the folder above.
    if at.join("partition").is_file() {
        if let Some(disk) = at.parent().filter(|disk| disk.join("stat").is_file()) {
            return Some(disk.to_path_buf());
        }
    }
    at.join("stat").is_file().then_some(at)
}

/// A `stat` line: reads completed, merged, sectors read, time; writes
/// completed, merged, sectors written, time; then more fields that newer
/// kernels add. Sectors are 512 bytes, whatever the device's.
pub fn parse_stat(line: &str) -> Option<Counters> {
    let fields: Vec<u64> = line
        .split_whitespace()
        .map(|field| field.parse().ok())
        .collect::<Option<_>>()?;
    if fields.len() < 7 {
        return None;
    }
    Some(Counters {
        ops: fields[0].saturating_add(fields[4]),
        read_bytes: fields[2].saturating_mul(512),
        written_bytes: fields[6].saturating_mul(512),
    })
}

#[cfg(test)]
#[path = "counters_tests.rs"]
mod tests;
