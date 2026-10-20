//! The walk of a stick's roots: the own folder, the folder at the top and
//! each named linked entry, giving every file's manifest path and where it
//! comes from.

use std::collections::BTreeSet;

use super::{DiskRoot, Source};
use crate::layout::{Root, StickLayout};
use crate::manifest::build::{for_each_regular_file, relative_path};
use crate::manifest::ManifestError;
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
pub(super) fn walk(layout: &StickLayout, recorded: &[OwnedFile]) -> Result<Walked, ManifestError> {
    let mut walked = Walked {
        files: Vec::new(),
        roots: Vec::new(),
        gone: Vec::new(),
        left_out: Vec::new(),
    };
    let mut left_out = BTreeSet::new();
    let mut at_top = |walked: &mut Walked, root: &Root| {
        let index = walked.roots.len();
        let index = u32::try_from(index).expect("fewer roots than u32::MAX");
        for_each_regular_file(&root.path, &mut |path| {
            let relative = relative_path(&root.path, path)?;
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
    at_top(&mut walked, layout.own())?;
    for root in layout.linked() {
        let name = root.name.clone().unwrap_or_default();
        if root.is_gone() {
            // Its files as recorded: the folder at the top's are those under
            // no named entry.
            let prefix = format!("{name}/");
            let carried = recorded.iter().filter(|file| {
                if root.at_top {
                    let first = file.path.split('/').next().unwrap_or_default();
                    !layout.is_named(first)
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
            at_top(&mut walked, root)?;
            continue;
        }
        let index = u32::try_from(walked.roots.len()).expect("fewer roots than u32::MAX");
        if root.is_dir {
            for_each_regular_file(&root.path, &mut |path| {
                let relative = format!("{name}/{}", relative_path(&root.path, path)?);
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
    walked.left_out = left_out.into_iter().collect();
    Ok(walked)
}
