//! The walk of a folder through [`SenderFiles`]: every regular file under
//! it, depth first in the order the folders list them, symbolic links never
//! followed, the files systems make passed over ([`is_ignored`]).

use std::io;
use std::path::{Path, PathBuf};

use super::{FileFacts, FileKind, SenderFiles};
use crate::manifest::ignore::is_ignored;

/// Walks `root` (followed if it is a link; a file is its own only file),
/// giving each regular file to `found` with its facts. A folder that cannot
/// be listed goes to `unreadable`, which stops the walk with an error or
/// passes it over. Blocking.
pub fn for_each_file<E>(
    files: &dyn SenderFiles,
    root: &Path,
    found: &mut dyn FnMut(&Path, &FileFacts) -> Result<(), E>,
    unreadable: &mut dyn FnMut(&Path, io::Error) -> Result<(), E>,
) -> Result<(), E> {
    let facts = match files.facts_followed(root) {
        Ok(facts) => facts,
        Err(error) => return unreadable(root, error),
    };
    match facts.kind {
        FileKind::File => return found(root, &facts),
        FileKind::Folder => {}
        _ => return Ok(()),
    }
    // Folders still to list, the next one last: depth first, each folder's
    // entries in its own order.
    let mut folders: Vec<std::vec::IntoIter<(PathBuf, FileFacts)>> = Vec::new();
    folders.push(listed(files, root, unreadable)?.into_iter());
    while let Some(entries) = folders.last_mut() {
        let Some((path, facts)) = entries.next() else {
            folders.pop();
            continue;
        };
        match facts.kind {
            FileKind::File => found(&path, &facts)?,
            FileKind::Folder => {
                let inside = listed(files, &path, unreadable)?;
                folders.push(inside.into_iter());
            }
            FileKind::Link | FileKind::Other => {
                // No path: on a phone it holds what the user chose.
                tracing::debug!("skipping a non-regular file");
            }
        }
    }
    Ok(())
}

/// The entries of `folder` the walk goes into: system files and folders
/// left out. One that cannot be listed is `unreadable`'s to decide; passed
/// over, it has none.
fn listed<E>(
    files: &dyn SenderFiles,
    folder: &Path,
    unreadable: &mut dyn FnMut(&Path, io::Error) -> Result<(), E>,
) -> Result<Vec<(PathBuf, FileFacts)>, E> {
    let entries = match files.entries(folder) {
        Ok(entries) => entries,
        Err(error) => {
            unreadable(folder, error)?;
            return Ok(Vec::new());
        }
    };
    Ok(entries
        .into_iter()
        .filter(|(name, _)| !name.to_str().is_some_and(is_ignored))
        .map(|(name, facts)| (folder.join(name), facts))
        .collect())
}
