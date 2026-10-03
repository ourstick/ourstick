//! A pass over a stick's folder (ADR 0007): the catalogue in parts, and a
//! version published at the end; the files' blobs go into suitcases only
//! when readers ask for them ([`super::keeper`]). Partial versions
//! are published as the catalogue grows, so readers start before it ends;
//! each keeps the files not read yet as the last pass left them
//! ([`super::partial`]), and one that changes nothing is not published.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::disk_turns::{Readers, StopOnDrop};
use super::partial::Previous;
use super::version::{Signed, Versions};
use super::{check_drive, drive_present, Owner, Publication, ShareError};
use crate::blocking::run_blocking;
use crate::cipher::cipher_for;
use crate::layout::StickLayout;
use crate::manifest::{name_for_manifest, Catalogue, Catalogued, Entry, Manifest};
use crate::store::{OwnedFile, StickKind};
use crate::transport::Publisher;

/// A part of the catalogue closes at this many files…
const PART_FILES: usize = 256;
/// …or once this many bytes were read, so the pass looks up often enough to
/// publish on time and to notice an ejected drive.
const PART_BYTES: u64 = 256 * 1024 * 1024;

/// When partial versions are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialTiming {
    /// The first one after the pass starts (an empty one comes at once if
    /// the stick has never been published, so it has a link).
    pub first_after: Duration,
    pub every: Duration,
}

/// ADR 0007: after 30 seconds, then every 3 minutes.
pub const PARTIAL_TIMING: PartialTiming = PartialTiming {
    first_after: Duration::from_secs(30),
    every: Duration::from_secs(3 * 60),
};

/// How a pass runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct PassOptions {
    /// Read every file, whatever was recorded.
    pub rescan: bool,
    /// Publish partial versions, as timed. `None`: only at the end.
    pub partial: Option<PartialTiming>,
}

/// What a pass does, for display.
#[derive(Debug)]
pub enum PassEvent<'a> {
    /// `done` of `total` files are catalogued.
    Progress { done: usize, total: usize },
    /// A partial version was published.
    Published(&'a Publication),
    /// OurStick began (`true`) or stopped giving way to another program
    /// using the disk (ADR 0013). Told at each change only.
    GivingWay(bool),
    /// At the start, if any: linked entries that are gone (their files are
    /// offered as last published), and top-level names of the stick's own
    /// folder left out because a linked entry has them (ADR 0008). Names
    /// only, never paths.
    Roots {
        gone: &'a [String],
        left_out: &'a [String],
    },
}

/// What a pass came to.
#[derive(Debug)]
pub struct PassOutcome {
    /// The complete version it published.
    pub publication: Publication,
    /// Files left out because they changed or went while they were read:
    /// another pass should follow soon.
    pub skipped: usize,
}

/// Runs one pass over the folder of the stick `owner` shares. Blocking work
/// runs on the blocking pool.
pub async fn run_pass(
    owner: &Owner<'_, impl Publisher>,
    options: PassOptions,
    on_event: &mut (dyn FnMut(PassEvent<'_>) + Send),
) -> Result<PassOutcome, ShareError> {
    let mut giving_way = false;
    let outcome = pass(owner, options, on_event, &mut giving_way).await;
    // However it ended, a pass that was giving way says it no longer is.
    if giving_way {
        on_event(PassEvent::GivingWay(false));
    }
    outcome
}

/// [`run_pass`]; `giving_way` is what it last told.
async fn pass(
    owner: &Owner<'_, impl Publisher>,
    options: PassOptions,
    on_event: &mut (dyn FnMut(PassEvent<'_>) + Send),
    giving_way: &mut bool,
) -> Result<PassOutcome, ShareError> {
    let stick_id = owner.signing_key.stick_id();
    let owned = {
        let store = owner.store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let cipher = cipher_for(owned.cipher, owner.stick_key);
    // ADR 0009: the stick's name travels in its manifest, for the reader's
    // list.
    let name = name_for_manifest(&owned.name);
    let root = owned.root_path.clone();
    // The app's stick is a drive: never publish an ejected one (an empty
    // mount point) as an empty stick.
    let is_drive = owned.kind == StickKind::App;
    if is_drive {
        let root = root.clone();
        run_blocking(move || check_drive(&root, &stick_id)).await?;
    }
    let recorded: Vec<OwnedFile> = {
        let store = owner.store.clone();
        run_blocking(move || store.owned_files(&stick_id)).await?
    };
    let layout = {
        let store = owner.store.clone();
        run_blocking(move || StickLayout::load(store.as_ref(), &stick_id)).await?
    };
    let mut catalogue = {
        let recorded = recorded.clone();
        let rescan = options.rescan;
        run_blocking(move || Catalogue::start_layout(&layout, recorded, rescan)).await?
    };
    if !catalogue.gone().is_empty() || !catalogue.left_out().is_empty() {
        on_event(PassEvent::Roots {
            gone: catalogue.gone(),
            left_out: catalogue.left_out(),
        });
    }
    let mut previous = Previous::new(recorded, &catalogue.paths());

    let versions = Versions {
        publisher: owner.publisher,
        store: owner.store.clone(),
        clock: owner.clock,
        signing_key: owner.signing_key,
        cipher: cipher.clone(),
    };
    let mut signed = Signed {
        last_signed_seq: owned.last_signed_seq,
        latest_record: owned.latest_record.clone(),
    };
    let timing = options.partial;
    let started = Instant::now();
    let mut next_partial = timing.map(|timing| started + timing.first_after);
    if timing.is_some() && owned.latest_record.is_none() {
        let publication = versions
            .publish(&mut signed, partial_manifest(&name, Vec::new()), 0)
            .await?;
        on_event(PassEvent::Published(&publication));
    }

    // Read through the disks' turns (ADR 0013), giving way to other
    // programs; a pause is cut short when the pass is dropped (its stick
    // stopped) or the stick's drive is gone.
    let stop_on_drop = StopOnDrop::default();
    let stopped = stop_on_drop.flag();
    let mut readers = {
        let root = root.clone();
        Some(Readers::new(owner.disks.clone(), false, move || {
            if stopped.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("the stick stopped"));
            }
            if is_drive && !drive_present(&root, &stick_id) {
                return Err(std::io::Error::other("the stick's drive is gone"));
            }
            Ok(())
        }))
    };

    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = 0;
    let mut reused = 0;
    while !catalogue.is_finished() {
        if is_drive {
            let marker_root = root.clone();
            if !run_blocking(move || drive_present(&marker_root, &stick_id)).await {
                return Err(ShareError::DriveMissing { path: root.clone() });
            }
        }
        let (returned, returned_readers, part) = {
            let cipher = cipher.clone();
            let mut readers = readers.take();
            run_blocking(move || {
                let part =
                    catalogue.next_part(cipher.as_ref(), PART_FILES, PART_BYTES, readers.as_mut());
                (catalogue, readers, part)
            })
            .await
        };
        catalogue = returned;
        readers = returned_readers;
        // A drive ejected during a pause says so, not "a file failed".
        if part.is_err() && is_drive {
            let marker_root = root.clone();
            if !run_blocking(move || drive_present(&marker_root, &stick_id)).await {
                return Err(ShareError::DriveMissing { path: root.clone() });
            }
        }
        let mut recorded = Vec::new();
        for item in part? {
            let file = match item {
                Catalogued::File(file) => file,
                Catalogued::Skipped { .. } => {
                    skipped += 1;
                    continue;
                }
            };
            reused += usize::from(!file.read);
            recorded.push(file.owned_file());
            entries.push(file.entry);
        }
        {
            let store = owner.store.clone();
            run_blocking(move || store.record_owned_files(&stick_id, &recorded)).await?;
        }
        on_event(PassEvent::Progress {
            done: catalogue.done(),
            total: catalogue.len(),
        });
        let now = readers.as_ref().is_some_and(Readers::giving_way);
        if now != *giving_way {
            *giving_way = now;
            on_event(PassEvent::GivingWay(now));
        }
        if let (Some(timing), Some(due)) = (timing, next_partial) {
            if Instant::now() >= due && !catalogue.is_finished() {
                let merged = previous.merged(&entries, catalogue.last_done());
                if previous.changes(&merged) {
                    let publication = versions
                        .publish(&mut signed, partial_manifest(&name, merged.clone()), reused)
                        .await?;
                    on_event(PassEvent::Published(&publication));
                    previous.published(merged);
                }
                next_partial = Some(Instant::now() + timing.every);
            }
        }
    }

    let paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    let manifest = Manifest {
        partial: false,
        name: name.clone(),
        entries,
    };
    let publication = versions.publish(&mut signed, manifest, reused).await?;
    // Only now, with the complete version out: what the pass did not keep
    // leaves the record.
    {
        let store = owner.store.clone();
        run_blocking(move || store.keep_only_owned_files(&stick_id, &paths)).await?;
    }
    Ok(PassOutcome {
        publication,
        skipped,
    })
}

fn partial_manifest(name: &str, entries: Vec<Entry>) -> Manifest {
    Manifest {
        partial: true,
        name: name.to_owned(),
        entries,
    }
}
