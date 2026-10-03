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
/// Less than this many times: fast, someone else is gone.
const FAST_RATIO: f64 = 1.5;
/// Slow windows in a row before OurStick gives way: one alone (a drive
/// waking up, a sector read again) is never enough.
const SLOW_IN_A_ROW: u32 = 2;
/// While giving way, a pause this many times the window: OurStick reads a
/// tenth of the time.
const PAUSE_FACTOR: u32 = 9;
const SHORTEST_PAUSE: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(30);
/// A recent best counts half as much (is twice as easy to match) after
/// this much reading time, so a slower stretch of the disk is not taken
/// for someone else for long.
const FADE: Duration = Duration::from_secs(60 * 60);
/// Files of at least this size on average make a window of big files (its
/// cost is its bytes); smaller ones, a window of small files (a seek each).
const BIG_FILE: u64 = 1 << 20;
/// Faster than any disk: read from memory (files just copied in). Such a
/// window is fast, but teaches no best.
const FASTEST_BYTES_PER_SECOND: f64 = 1e9;
const FASTEST_FILE: Duration = Duration::from_micros(200);

/// What judges how a disk is shared (ADR 0013): the timing of OurStick's
/// own windows ([`DiskShare`]) today; the system's disk counters could
/// take its place without changing its callers.
pub trait DiskLoad: Send {
    /// A window read: the pause before the next (zero at full speed).
    fn after_window(&mut self, files: u64, bytes: u64, took: Duration) -> Duration;
    /// The pause after reading that is not judged (a second read).
    fn pause_for(&self, took: Duration) -> Duration;
    /// Whether OurStick gives way now.
    fn giving_way(&self) -> bool;
}

impl DiskLoad for DiskShare {
    fn after_window(&mut self, files: u64, bytes: u64, took: Duration) -> Duration {
        DiskShare::after_window(self, files, bytes, took)
    }

    fn pause_for(&self, took: Duration) -> Duration {
        DiskShare::pause_for(self, took)
    }

    fn giving_way(&self) -> bool {
        DiskShare::giving_way(self)
    }
}

/// A recent window of one kind, per unit (a file, or a byte), in seconds,
/// and when it was seen (in reading time).
#[derive(Debug, Clone, Copy)]
struct Seen {
    per_unit: f64,
    seen_at: Duration,
}

impl Seen {
    /// Its time per unit, faded by the reading done since.
    fn faded(&self, reading: Duration) -> f64 {
        let since = (reading - self.seen_at).as_secs_f64() / FADE.as_secs_f64();
        self.per_unit * 2f64.powf(since)
    }
}

/// The two best recent windows of one kind. What a window should take is
/// the second best: one window read partly from memory (a few files just
/// copied in) is not a speed the disk keeps.
#[derive(Debug, Default, Clone, Copy)]
struct Best {
    first: Option<Seen>,
    second: Option<Seen>,
}

impl Best {
    fn expected_per_unit(&self, reading: Duration) -> Option<f64> {
        self.second.or(self.first).map(|seen| seen.faded(reading))
    }

    fn learn(&mut self, seen: Seen, reading: Duration) {
        let beats =
            |other: Option<Seen>| other.is_none_or(|other| seen.per_unit < other.faded(reading));
        if beats(self.first) {
            self.second = self.first;
            self.first = Some(seen);
        } else if beats(self.second) {
            self.second = Some(seen);
        }
    }
}

/// One disk's judgement (ADR 0013, rules 2–4).
#[derive(Debug, Default)]
pub struct DiskShare {
    per_file: Best,
    per_byte: Best,
    /// The sum of every window's time.
    reading: Duration,
    slow_in_a_row: u32,
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
        self.reading += took;
        let seconds = took.as_secs_f64();
        // A window inside a big file begins no file: it is of big files.
        let big = files == 0 || bytes / files >= BIG_FILE;
        let (units, best) = if big {
            (bytes, &mut self.per_byte)
        } else {
            (files, &mut self.per_file)
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
            .expected_per_unit(self.reading)
            .map(|per_unit| per_unit * units as f64);
        let ratio = match expected {
            Some(expected) if expected > 0.0 && !from_memory => seconds / expected,
            // Nothing to judge it by yet, or read from memory: fast.
            _ => 0.0,
        };
        // A slow window teaches nothing either: someone else's reading is
        // not the disk's speed.
        if !from_memory && ratio <= SLOW_RATIO {
            let seen = Seen {
                per_unit,
                seen_at: self.reading,
            };
            best.learn(seen, self.reading);
        }
        let was = self.giving_way;
        if ratio > SLOW_RATIO {
            self.slow_in_a_row += 1;
            if self.slow_in_a_row >= SLOW_IN_A_ROW {
                self.giving_way = true;
            }
        } else {
            self.slow_in_a_row = 0;
            if ratio < FAST_RATIO {
                self.giving_way = false;
            }
        }
        if was != self.giving_way {
            tracing::debug!(
                giving_way = self.giving_way,
                window_ms = took.as_millis() as u64,
                expected_ms = expected.map(|expected| (expected * 1000.0) as u64),
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
#[path = "disk_share_tests.rs"]
mod tests;
