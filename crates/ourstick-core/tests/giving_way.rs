//! ADR 0013: a pass gives way to another program using the disk, and says
//! so, when its reads get far slower, and goes back to full speed when they
//! are fast again. The disk's slowness is a fake timer's.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::Owner;
use ourstick_core::share::disk_turns::{Disks, Timer};
use ourstick_core::share::{run_pass, PassEvent, PassOptions};
use ourstick_core::watcher::Owner as Publishing;

/// Each look at the clock moves it on by a step: a read of a small file
/// takes one step, 1 ms (a disk), or 10 ms while `slow` (someone else
/// reading too). Sleeping moves it on at once.
#[derive(Default)]
struct SteppingTimer {
    micros: AtomicU64,
    slow: AtomicBool,
}

impl Timer for SteppingTimer {
    fn elapsed(&self) -> Duration {
        let step = if self.slow.load(Ordering::Relaxed) {
            10_000
        } else {
            1_000
        };
        Duration::from_micros(self.micros.fetch_add(step, Ordering::Relaxed) + step)
    }

    fn sleep(&self, duration: Duration) {
        self.micros
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pass_gives_way_while_reads_are_slow_and_says_so() {
    // Three parts of 256 small files: fast, slow, fast.
    let files: Vec<(String, Vec<u8>)> = (0..768)
        .map(|index| (format!("f{index:04}.bin"), vec![index as u8; 4096]))
        .collect();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::new(&listed).await;
    let timer = Arc::new(SteppingTimer::default());
    let owning = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        disks: Arc::new(Disks::new(timer.clone())),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };

    let mut told = Vec::new();
    let mut parts = 0;
    let outcome = run_pass(&owning, PassOptions::default(), &mut |event| match event {
        PassEvent::Progress { .. } => {
            parts += 1;
            // The second part's reads are slow, the third's fast again.
            timer.slow.store(parts == 1, Ordering::Relaxed);
        }
        PassEvent::GivingWay(now) => told.push((parts, now)),
        _ => {}
    })
    .await
    .unwrap();

    assert_eq!(outcome.publication.manifest.entries.len(), 768);
    // Told after the slow part, and when fast again: once each.
    assert_eq!(told, vec![(2, true), (3, false)]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_pass_that_fails_while_giving_way_says_it_no_longer_is() {
    use std::os::unix::fs::PermissionsExt;
    let files: Vec<(String, Vec<u8>)> = (0..768)
        .map(|index| (format!("f{index:04}.bin"), vec![index as u8; 4096]))
        .collect();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let owner = Owner::new(&listed).await;
    // A file of the third part cannot be read: the pass fails there.
    let unreadable = owner.folder.path().join("f0600.bin");
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    let timer = Arc::new(SteppingTimer::default());
    let owning = Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        disks: Arc::new(Disks::new(timer.clone())),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let mut told = Vec::new();
    let mut parts = 0;
    let outcome = run_pass(&owning, PassOptions::default(), &mut |event| match event {
        PassEvent::Progress { .. } => {
            parts += 1;
            timer.slow.store(parts >= 1, Ordering::Relaxed);
        }
        PassEvent::GivingWay(now) => told.push((parts, now)),
        _ => {}
    })
    .await;
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(outcome.is_err());
    // Giving way after the slow part; then the failure, and the end of it.
    assert_eq!(told, vec![(2, true), (2, false)]);
}
