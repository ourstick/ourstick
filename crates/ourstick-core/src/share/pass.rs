//! A pass over a stick's folder (ADR 0007): the catalogue in parts, and a
//! version published at the end; the files' blobs go into suitcases only
//! when readers ask for them ([`super::keeper`]). Partial versions
//! are published as the catalogue grows, so readers start before it ends;
//! each keeps the files not read yet as the last pass left them
//! ([`super::partial`]), and one that changes nothing is not published.

use std::time::{Duration, Instant};

use super::partial::Previous;
use super::version::{Signed, Versions};
use super::{check_drive, drive_present, Owner, Publication, ShareError};
use crate::blocking::run_blocking;
use crate::cipher::cipher_for;
use crate::manifest::{Catalogue, Catalogued, Entry, Manifest};
use crate::store::OwnedFile;
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
    let stick_id = owner.signing_key.stick_id();
    let owned = {
        let store = owner.store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let cipher = cipher_for(owned.cipher, owner.stick_key);
    let root = owned.root_path.clone();
    if let Some(size) = owned.drive_size {
        // Never publish an ejected drive (an empty mount point) as an empty
        // stick, nor a stick that does not fit.
        let root = root.clone();
        let used = run_blocking(move || check_drive(&root, &stick_id)).await?;
        if used > size {
            return Err(ShareError::StickFull { size, used });
        }
    }
    let recorded: Vec<OwnedFile> = {
        let store = owner.store.clone();
        run_blocking(move || store.owned_files(&stick_id)).await?
    };
    let known = if options.rescan {
        Vec::new()
    } else {
        recorded.clone()
    };
    let mut catalogue = {
        let root = root.clone();
        run_blocking(move || Catalogue::start(&root, known)).await?
    };
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
            .publish(&mut signed, partial_manifest(Vec::new()), 0)
            .await?;
        on_event(PassEvent::Published(&publication));
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = 0;
    let mut reused = 0;
    while !catalogue.is_finished() {
        if owned.drive_size.is_some() {
            let marker_root = root.clone();
            if !run_blocking(move || drive_present(&marker_root, &stick_id)).await {
                return Err(ShareError::DriveMissing { path: root.clone() });
            }
        }
        let (returned, part) = {
            let cipher = cipher.clone();
            run_blocking(move || {
                let part = catalogue.next_part(cipher.as_ref(), PART_FILES, PART_BYTES);
                (catalogue, part)
            })
            .await
        };
        catalogue = returned;
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
        if let (Some(timing), Some(due)) = (timing, next_partial) {
            if Instant::now() >= due && !catalogue.is_finished() {
                let merged = previous.merged(&entries, catalogue.last_done());
                if previous.changes(&merged) {
                    let publication = versions
                        .publish(&mut signed, partial_manifest(merged.clone()), reused)
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
        name: String::new(),
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

fn partial_manifest(entries: Vec<Entry>) -> Manifest {
    Manifest {
        partial: true,
        name: String::new(),
        entries,
    }
}
