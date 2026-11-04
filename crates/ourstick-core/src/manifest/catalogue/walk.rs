//! The walk of a stick's roots: the own folder, the folder at the top and
//! each named linked entry, giving every file's manifest path and where it
//! comes from.

use std::collections::BTreeSet;
use std::path::Path;

use super::{DiskRoot, Source};
use crate::layout::{Root, StickLayout};
use crate::manifest::build::{for_each_regular_file, relative_path};
use crate::manifest::ManifestError;
use crate::sender_files::SenderFiles;
use crate::store::OwnedFile;

/// What the walk found.
pub(super) struct Walked {
    /// Unsorted.
    pub files: Vec<(String, Source)>,
    pub roots: Vec<DiskRoot>,
    /// The linked entries that are gone, by name.
    pub gone: Vec<String>,
    /// Top-level names of the own folder, or of the folder at the top, left
    /// out because a named linked entry has them.
    pub left_out: Vec<String>,
}

/// Walks every root of `layout`; a linked entry that is gone has its files
/// carried as `recorded`. Blocking.
pub(super) fn walk(
    files: &dyn SenderFiles,
    layout: &StickLayout,
    recorded: &[OwnedFile],
) -> Result<Walked, ManifestError> {
    let mut walked = Walked {
        files: Vec::new(),
        roots: Vec::new(),
        gone: Vec::new(),
        left_out: Vec::new(),
    };
    let mut left_out = BTreeSet::new();
    let mut walk_folder = |walked: &mut Walked, root: &Root| {
        let index = walked.roots.len();
        let index = u32::try_from(index).expect("fewer roots than u32::MAX");
        for_each_regular_file(files, &root.path, &mut |path, _| {
            let Some(relative) = sendable(&root.path, path)? else {
                return Ok(());
            };
            count(&walked.files)?;
            let top = relative.split('/').next().unwrap_or_default();
            if layout.is_shadowed(top) {
                left_out.insert(top.to_owned());
            } else {
                walked.files.push((relative, Source::Disk(index)));
            }
            Ok(())
        })?;
        walked.roots.push(DiskRoot {
            path: root.path.clone(),
            name: None,
            is_dir: true,
        });
        Ok::<_, ManifestError>(())
    };
    walk_folder(&mut walked, layout.own())?;
    // What the own folder gives is never carried again for the top.
    let own: std::collections::HashSet<String> =
        walked.files.iter().map(|(path, _)| path.clone()).collect();
    for root in layout.linked() {
        let name = root.name.clone().unwrap_or_default();
        if root.is_gone(files) {
            // Its files as recorded: the folder at the top's are those under
            // no named entry, by its exact name (a name only shadowed, in
            // another case, was never recorded).
            let prefix = format!("{name}/");
            let carried = recorded.iter().filter(|file| {
                if root.at_top {
                    let first = file.path.split('/').next().unwrap_or_default();
                    !layout.is_named(first) && !own.contains(&file.path)
                } else {
                    file.path == name || file.path.starts_with(&prefix)
                }
            });
            walked
                .files
                .extend(carried.map(|file| (file.path.clone(), Source::Carried)));
            walked.gone.push(name);
            continue;
        }
        if root.at_top {
            walk_folder(&mut walked, root)?;
            continue;
        }
        let index = u32::try_from(walked.roots.len()).expect("fewer roots than u32::MAX");
        if root.is_dir {
            for_each_regular_file(files, &root.path, &mut |path, _| {
                let Some(inside) = sendable(&root.path, path)? else {
                    return Ok(());
                };
                count(&walked.files)?;
                let relative = format!("{name}/{inside}");
                walked.files.push((relative, Source::Disk(index)));
                Ok(())
            })?;
        } else {
            walked.files.push((name.clone(), Source::Disk(index)));
        }
        walked.roots.push(DiskRoot {
            path: root.path.clone(),
            name: Some(name),
            is_dir: root.is_dir,
        });
    }
    // One path from two roots (the own folder and the folder at the top):
    // the own folder's, walked first, is the one sent; the other is left
    // out, as a name a linked entry has is.
    walked.files.sort_by(|a, b| a.0.cmp(&b.0));
    let before = walked.files.len();
    walked.files.dedup_by(|later, first| {
        let same = later.0 == first.0;
        if same {
            left_out.insert(later.0.split('/').next().unwrap_or_default().to_owned());
        }
        same
    });
    if walked.files.len() < before {
        tracing::warn!("files at one path in two places: the own folder's is sent");
    }
    walked.left_out = left_out.into_iter().collect();
    Ok(walked)
}

/// A file's path in the stick, or `None` for one that cannot be sent (a
/// name that is not UTF-8, holds a `\`, or a path too long or too deep):
/// that file is left out with a warning, never the whole stick.
fn sendable(root: &Path, path: &Path) -> Result<Option<String>, ManifestError> {
    match relative_path(root, path) {
        Ok(relative) => Ok(Some(relative)),
        Err(ManifestError::InvalidPath(_) | ManifestError::NonUtf8Path(_)) => {
            tracing::warn!("a file left out of a stick: its name cannot be sent");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Fails once a walk found more files than a stick may have, before it
/// reads more of a folder that is far too big.
fn count(files: &[(String, Source)]) -> Result<(), ManifestError> {
    if files.len() >= crate::manifest::MAX_ENTRIES {
        return Err(ManifestError::TooManyFiles {
            count: files.len() + 1,
        });
    }
    Ok(())
}
