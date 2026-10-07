//! A disk judged by the kernel's counters (ADR 0013, amendment 1): who
//! else moved bytes on it, or kept it busy, while OurStick read it.

use std::time::Duration;

use super::{DiskShare, ReaderKind, SLOW_IN_A_ROW, SLOW_RATIO};
use crate::disk::counters::Counters;

/// Someone else moved at least this much in a window (what the system
/// reads ahead for OurStick's own files stays under it)…
const OTHERS_FLOOR: u64 = 4 << 20;
/// …and at least this part of what OurStick took from the disk.
const OTHERS_SHARE: u64 = 4;
/// A slow window is someone else's when the disk did this many operations
/// more than OurStick's own, plus two a file begun (its metadata)…
const OTHERS_OPS: u64 = 64;
/// …OurStick's own counted as one per this many bytes it took from the
/// disk. Its read calls are no measure: the kernel merges small ones and
/// reads ahead. Its requests are rarely smaller, so its own are not taken
/// for someone else's; larger ones only make others look fewer.
const OURS_PER_OP: u64 = 64 << 10;

/// What the kernel saw of a window: the disk's traffic while OurStick read
/// it, and what OurStick read (some from memory).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    pub disk: Counters,
    pub ours: u64,
}

impl DiskShare {
    /// A window the kernel's counters saw: someone else is there when, two
    /// windows in a row, others moved a quarter of what OurStick took from
    /// the disk and 4 MB, or the window was slow and the disk did
    /// operations OurStick did not; gone at the first window with neither.
    /// The second test needs a window of at least 4 MB from the disk (one
    /// from memory tells nothing of speed). Speeds are learned from windows
    /// with no one else.
    pub fn after_window_seen(
        &mut self,
        kind: ReaderKind,
        files: u64,
        bytes: u64,
        took: Duration,
        seen: Seen,
    ) -> Duration {
        let ours = seen.ours.min(seen.disk.read_bytes);
        let all = seen.disk.read_bytes.saturating_add(seen.disk.written_bytes);
        let others = all.saturating_sub(ours);
        let moved = others >= OTHERS_FLOOR && others.saturating_mul(OTHERS_SHARE) >= ours;
        let from_disk = ours >= OTHERS_FLOOR;
        let ours_ops = ours / OURS_PER_OP + 2 * files;
        let others_ops = seen.disk.ops.saturating_sub(ours_ops);
        let ratio = if from_disk {
            self.ratio(kind, files, bytes, took)
        } else {
            None
        };
        let slow = ratio.is_some_and(|ratio| ratio > SLOW_RATIO);
        let busy = slow && others_ops >= OTHERS_OPS;
        let someone = moved || busy;
        if !someone && from_disk {
            self.learn(kind, files, bytes, took);
        }
        let was = self.giving_way;
        if someone {
            self.others_in_a_row += 1;
            if self.others_in_a_row >= SLOW_IN_A_ROW {
                self.giving_way = true;
            }
        } else {
            self.others_in_a_row = 0;
            self.giving_way = false;
        }
        if was != self.giving_way {
            tracing::debug!(
                giving_way = self.giving_way,
                window_ms = took.as_millis() as u64,
                ratio = ratio.unwrap_or(0.0),
                ours,
                others,
                disk_ops = seen.disk.ops,
                ours_ops,
                files,
                "the disk is shared differently (kernel's counters)"
            );
        }
        self.pause(took)
    }
}
