//! Giving way to the user's disk (ADR 0013): from how long OurStick's own
//! reads take, whether someone else is using the disk, and how long to
//! pause after a window of reading. Pure: it is given each window's files,
//! bytes and time, and keeps no clock of its own.

use std::time::Duration;

/// A window closes after this many bytes read…
pub const WINDOW_BYTES: u64 = 64 << 20;
/// …or this many files, whichever comes first.
pub const WINDOW_FILES: u64 = 64;

/// A window took more than this many times what it should have: slow.
const SLOW_RATIO: f64 = 3.0;
/// Less than this many times: someone else is gone. Above the inner tracks
/// of a disk (up to twice as slow as its outer ones), so OurStick alone on
/// them is not taken for someone else still there.
const FAST_RATIO: f64 = 2.5;
/// Slow windows in a row before OurStick gives way: one alone (a drive
/// waking up, a sector read again) is never enough.
const SLOW_IN_A_ROW: u32 = 2;
/// While giving way, a pause this many times the window: OurStick reads a
/// tenth of the time.
const PAUSE_FACTOR: u32 = 9;
const SHORTEST_PAUSE: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(30);
/// What a window should take is the median of this many recent windows
/// of its kind, learned while OurStick is not giving way: one window read
/// ahead by the system, or partly from memory, does not move it.
const RECENT: usize = 16;
/// Nothing is judged before this many windows of a kind: the first ones
/// alone cannot tell the system reading ahead from someone else reading.
const FIRST_WINDOWS: usize = 3;
/// Slow this many windows in a row while giving way, the slow speed is the
/// disk's now (a slower disk, not someone else): it is learned.
const RELEARN_AFTER: u32 = 64;
/// Files of at least this size on average make a window of big files (its
/// cost is its bytes); smaller ones, a window of small files (a seek each).
const BIG_FILE: u64 = 1 << 20;
/// Faster than any disk: read from memory (files just copied in, or read
/// ahead by the system). Such a window changes nothing and teaches no best.
const FASTEST_BYTES_PER_SECOND: f64 = 1e9;
const FASTEST_FILE: Duration = Duration::from_micros(200);

/// What judges how a disk is shared (ADR 0013): the timing of OurStick's
/// own windows ([`DiskShare`]) today; the system's disk counters could
/// take its place without changing its callers.
pub trait DiskLoad: Send {
    /// A window read by a reader of `kind`: the pause before the next
    /// (zero at full speed).
    fn after_window(
        &mut self,
        kind: ReaderKind,
        files: u64,
        bytes: u64,
        took: Duration,
    ) -> Duration;
    /// The pause after reading that is not judged (a second read).
    fn pause_for(&self, took: Duration) -> Duration;
    /// Whether OurStick gives way now.
    fn giving_way(&self) -> bool;
}

impl DiskLoad for DiskShare {
    fn after_window(
        &mut self,
        kind: ReaderKind,
        files: u64,
        bytes: u64,
        took: Duration,
    ) -> Duration {
        DiskShare::after_window_of(self, kind, files, bytes, took)
    }

    fn pause_for(&self, took: Duration) -> Duration {
        DiskShare::pause_for(self, took)
    }

    fn giving_way(&self) -> bool {
        DiskShare::giving_way(self)
    }
}

/// The recent windows of one kind, per unit (a file, or a byte), in
/// seconds, newest last.
#[derive(Debug, Default, Clone)]
struct Recent(std::collections::VecDeque<f64>);

impl Recent {
    /// The median of the recent windows; none before [`FIRST_WINDOWS`].
    fn expected_per_unit(&self) -> Option<f64> {
        let mut values: Vec<f64> = self.0.iter().copied().collect();
        if values.len() < FIRST_WINDOWS {
            return None;
        }
        values.sort_by(f64::total_cmp);
        Some(values[values.len() / 2])
    }

    fn learn(&mut self, per_unit: f64) {
        if self.0.len() == RECENT {
            self.0.pop_front();
        }
        self.0.push_back(per_unit);
    }
}

/// What kind of reader read a window: each kind's speed is learned apart,
/// since they cost a disk differently (ADR 0017, amendment 1).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReaderKind {
    /// Whole files, from their start (a catalogue, the files keeper).
    #[default]
    Folder,
    /// Parts of files readers asked for, several at once.
    Transfers,
}

/// What one kind of reader's windows took, per file and per byte.
#[derive(Debug, Default)]
struct Learned {
    per_file: Recent,
    per_byte: Recent,
}

/// One disk's judgement (ADR 0013, rules 2–4): its speed learned per kind
/// of reader, and one judgement (giving way or not) for all of them.
#[derive(Debug, Default)]
pub struct DiskShare {
    folder: Learned,
    transfers: Learned,
    slow_in_a_row: u32,
    /// Windows in a row not fast while giving way.
    held_back: u32,
    giving_way: bool,
}

impl DiskShare {
    /// Whether OurStick is giving way now (someone else uses the disk).
    pub fn giving_way(&self) -> bool {
        self.giving_way
    }

    /// A window of `files` files (those begun in it) and `bytes` bytes read
    /// in `took`: the pause before the next one (zero at full speed).
    pub fn after_window(&mut self, files: u64, bytes: u64, took: Duration) -> Duration {
        self.after_window_of(ReaderKind::Folder, files, bytes, took)
    }

    /// A window of a reader of `kind`, judged by that kind's speed.
    pub fn after_window_of(
        &mut self,
        kind: ReaderKind,
        files: u64,
        bytes: u64,
        took: Duration,
    ) -> Duration {
        let seconds = took.as_secs_f64();
        // A window inside a big file begins no file: it is of big files.
        let big = files == 0 || bytes / files >= BIG_FILE;
        let learned = match kind {
            ReaderKind::Folder => &mut self.folder,
            ReaderKind::Transfers => &mut self.transfers,
        };
        let (units, best) = if big {
            (bytes, &mut learned.per_byte)
        } else {
            (files, &mut learned.per_file)
        };
        if units == 0 {
            return self.pause(took);
        }
        let per_unit = seconds / units as f64;
        let from_memory = if big {
            bytes as f64 > FASTEST_BYTES_PER_SECOND * seconds
        } else {
            per_unit < FASTEST_FILE.as_secs_f64()
        };
        let expected = best
            .expected_per_unit()
            .map(|per_unit| per_unit * units as f64);
        // Read from memory (files just copied in, or what the system read
        // ahead of OurStick): it tells nothing of the disk, and changes
        // nothing. Measured: the end of a part, read ahead, came at "0 ms"
        // and kept undoing the giving way.
        if from_memory {
            return self.pause(took);
        }
        let Some(expected) = expected.filter(|expected| *expected > 0.0) else {
            // Nothing to judge it by yet (the first windows of a kind, as
            // when a pass reaches small files after big ones): it changes
            // nothing, and is learned only while not giving way (else it is
            // someone else's speed), or once held back for long.
            if self.giving_way {
                self.held_back += 1;
            }
            if !self.giving_way || self.held_back >= RELEARN_AFTER {
                best.learn(per_unit);
            }
            return self.pause(took);
        };
        let ratio = seconds / expected;
        // Learned only while not giving way, and not from a slow window:
        // someone else's reading is not the disk's speed. Unless it lasts:
        // held back for long, the slower speed is the disk's.
        let fast = ratio < FAST_RATIO;
        if self.giving_way && !fast {
            self.held_back += 1;
        } else {
            self.held_back = 0;
        }
        if (!self.giving_way && ratio <= SLOW_RATIO) || self.held_back >= RELEARN_AFTER {
            best.learn(per_unit);
        }
        let was = self.giving_way;
        if ratio > SLOW_RATIO {
            self.slow_in_a_row += 1;
            if self.slow_in_a_row >= SLOW_IN_A_ROW {
                self.giving_way = true;
            }
        } else {
            self.slow_in_a_row = 0;
            if fast {
                self.giving_way = false;
            }
        }
        if was != self.giving_way {
            tracing::debug!(
                giving_way = self.giving_way,
                window_ms = took.as_millis() as u64,
                expected_ms = (expected * 1000.0) as u64,
                files,
                bytes,
                "the disk is shared differently"
            );
        }
        self.pause(took)
    }

    /// The pause after `took` of reading that is not judged (a second
    /// read of a file): as after a window, without judging it.
    pub fn pause_for(&self, took: Duration) -> Duration {
        self.pause(took)
    }

    fn pause(&self, took: Duration) -> Duration {
        if self.giving_way {
            (took * PAUSE_FACTOR).clamp(SHORTEST_PAUSE, LONGEST_PAUSE)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
#[path = "share_tests.rs"]
mod tests;
