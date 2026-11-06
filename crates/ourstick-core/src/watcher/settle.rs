//! Whether a folder has settled: no file under it changes size or times
//! over a while, so a pass does not publish a file half written.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::blocking::run_blocking;
use crate::manifest::build::for_each_regular_file;
use crate::manifest::{ManifestError, MAX_ENTRIES};
use crate::sender_files::SenderFiles;

/// What the files under some roots look like: their count and a sum of a
/// hash of each one's path and stamp, the same whatever order the walk
/// meets them in. Only the fingerprint is kept, never the list.
#[derive(Debug, Default, PartialEq, Eq)]
struct Fingerprint {
    count: usize,
    sum: [u64; 4],
}

impl Fingerprint {
    fn add(&mut self, path: &Path, stamp: &crate::manifest::FileStamp) {
        let mut hasher = blake3::Hasher::new();
        let path = path.as_os_str().as_encoded_bytes();
        hasher.update(&(path.len() as u64).to_le_bytes());
        hasher.update(path);
        for number in [stamp.size, stamp.mtime_ns, stamp.ctime_ns] {
            hasher.update(&number.to_le_bytes());
        }
        let hash = hasher.finalize();
        let (words, _) = hash.as_bytes().as_chunks::<8>();
        for (lane, word) in self.sum.iter_mut().zip(words) {
            *lane = lane.wrapping_add(u64::from_le_bytes(*word));
        }
        self.count += 1;
    }
}

/// The fingerprint of every file under each of `roots` (a root may be one
/// file), or `None` if one cannot be read right now (e.g. a file vanished
/// during the walk). Past [`MAX_ENTRIES`] files the walk stops, and the
/// same "too many" fingerprint comes back each time: the folder counts as
/// settled, and the pass says why it cannot take it.
fn fingerprint(files: &dyn SenderFiles, roots: &[PathBuf]) -> Option<Fingerprint> {
    let mut all = Fingerprint::default();
    for root in roots {
        let facts = files.facts_followed(root).ok()?;
        if facts.is_file() {
            all.add(root, &facts.stamp);
            continue;
        }
        let walked = for_each_regular_file(files, root, &mut |path, facts| {
            if all.count >= MAX_ENTRIES {
                return Err(ManifestError::TooManyFiles {
                    count: all.count + 1,
                });
            }
            all.add(path, &facts.stamp);
            Ok(())
        });
        match walked {
            Ok(()) => {}
            Err(ManifestError::TooManyFiles { .. }) => {
                return Some(Fingerprint {
                    count: usize::MAX,
                    sum: [0; 4],
                })
            }
            Err(_) => return None,
        }
    }
    Some(all)
}

/// True if no file under `root` changes size or times over `settle`.
pub async fn is_settled(files: Arc<dyn SenderFiles>, root: &Path, settle: Duration) -> bool {
    are_settled(files, vec![root.to_path_buf()], settle).await
}

/// [`is_settled`] for several roots at once.
pub async fn are_settled(
    files: Arc<dyn SenderFiles>,
    roots: Vec<PathBuf>,
    settle: Duration,
) -> bool {
    let read = |roots: Vec<PathBuf>| {
        let files = files.clone();
        run_blocking(move || Ok::<_, std::io::Error>(fingerprint(files.as_ref(), &roots)))
    };
    let Ok(Some(before)) = read(roots.clone()).await else {
        return false;
    };
    tokio::time::sleep(settle).await;
    matches!(read(roots).await, Ok(Some(after)) if after == before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::FileStamp;

    #[test]
    fn the_fingerprint_does_not_depend_on_order_but_on_every_fact() {
        let stamp = |size| FileStamp {
            size,
            mtime_ns: 1,
            ctime_ns: 2,
        };
        let (a, b) = (Path::new("a"), Path::new("b"));
        let mut one = Fingerprint::default();
        one.add(a, &stamp(1));
        one.add(b, &stamp(2));
        let mut other = Fingerprint::default();
        other.add(b, &stamp(2));
        other.add(a, &stamp(1));
        assert_eq!(one, other);
        let mut grown = Fingerprint::default();
        grown.add(a, &stamp(1));
        grown.add(b, &stamp(3));
        assert_ne!(one, grown);
        let mut renamed = Fingerprint::default();
        renamed.add(a, &stamp(1));
        renamed.add(Path::new("c"), &stamp(2));
        assert_ne!(one, renamed);
    }
}
