//! The turns' clock, and a way for a dropped task to stop waiting.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// A clock that never goes back, and a way to wait (blocking): real in the
/// app and the CLI, a fake in tests. Not the core's `Clock`, which tells
/// wall time in milliseconds for records and can jump.
pub trait Timer: Send + Sync {
    /// Time since some fixed start.
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

/// The system's monotonic clock.
#[derive(Debug)]
pub struct SystemTimer(Instant);

impl Default for SystemTimer {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Timer for SystemTimer {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A flag set when this is dropped: a task whose future is dropped (its
/// stick stopped) stops waiting for a disk, through its `go_on`.
#[derive(Debug, Default)]
pub struct StopOnDrop(Arc<std::sync::atomic::AtomicBool>);

impl StopOnDrop {
    /// Whether the task was dropped, for its `go_on`.
    pub fn flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.0.clone()
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
